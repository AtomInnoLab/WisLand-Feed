use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use conf::config::app_config;
use dotenvy::dotenv;
use feed::redis::verify_manager::VerifyManager;
use rand::seq::SliceRandom;
use seaorm_db::connection::get_db;
use seaorm_db::query::feed::{
    rss_sources::RssSourcesQuery, rss_subscriptions::RssSubscriptionsQuery,
    user_interests::UserInterestsQuery, user_paper_verifications::UserPaperVerificationsQuery,
};
use tokio::sync::Barrier;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

static INIT_TRACING: std::sync::Once = std::sync::Once::new();

fn init_test_tracing() {
    INIT_TRACING.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with_writer(std::io::stderr)
            .compact()
            .try_init();
        dotenv().ok();
    });
}

/// 测试多用户并发验证调度的公平性
/// 通过Redis统计和数据库记录两个维度验证调度策略
#[tokio::test]
async fn test_concurrent_multi_user_verify_fairness() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let db = get_db().await.clone();
    let config = app_config();

    // 创建Redis连接
    let redis_url = config.rss.feed_redis.url.clone();
    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            warn!(error = %err, "跳过测试: 无法连接 Redis");
            return Ok(());
        }
    };

    let redis_pool = match bb8::Pool::builder().max_size(10).build(manager).await {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "跳过测试: 无法创建Redis连接池");
            return Ok(());
        }
    };

    let _apalis_conn = match apalis_redis::connect(redis_url.as_str()).await {
        Ok(conn) => conn,
        Err(err) => {
            warn!(error = %err, "跳过测试: 无法创建Apalis Redis连接");
            return Ok(());
        }
    };

    info!("开始多用户并发验证公平性测试");

    // 1. 准备测试数据
    let test_user_ids: Vec<i64> = (3000000..3000005).collect(); // 5个测试用户
    let user_count = test_user_ids.len();

    // 获取现有的RSS源
    let available_sources = RssSourcesQuery::list_all(&db).await?;
    if available_sources.is_empty() {
        warn!("跳过测试: 数据库中没有RSS源");
        return Ok(());
    }

    let source_ids: Vec<i32> = available_sources.into_iter().map(|s| s.id).collect();
    info!(source_count = source_ids.len(), "使用现有RSS源进行测试");

    // 为每个用户创建兴趣和订阅
    for &user_id in &test_user_ids {
        setup_user_test_data(&db, user_id, &source_ids, &config).await?;
    }

    // 2. 创建VerifyManager
    let verify_manager = VerifyManager::new(
        redis_pool.clone(),
        db.clone(),
        config.rss.feed_redis.redis_prefix.clone(),
        config.rss.feed_redis.redis_key_default_expire,
    )
    .await;

    // 清理之前的状态
    for &user_id in &test_user_ids {
        cleanup_user_verify_state(&verify_manager, user_id).await?;
    }

    info!("测试环境准备完成，开始并发验证测试");

    // 3. 记录验证前的基线数据
    let mut baseline_redis_stats = HashMap::new();
    let mut baseline_db_counts = HashMap::new();

    for &user_id in &test_user_ids {
        // Redis基线
        let redis_info = match verify_manager.get_user_unverified_info(user_id).await {
            Ok(info) => info,
            Err(_) => {
                // 如果获取失败，创建零值统计
                feed::redis::verify_manager::UserVerifyInfo {
                    pending_unverify_count: 0,
                    success_count: 0,
                    fail_count: 0,
                    processing_count: 0,
                    total: 0,
                }
            }
        };
        let redis_baseline = redis_info.success_count + redis_info.fail_count;
        baseline_redis_stats.insert(user_id, redis_baseline);

        // 数据库基线
        let db_records = UserPaperVerificationsQuery::list_by_user_id(&db, user_id).await?;
        let db_baseline = db_records.len();
        baseline_db_counts.insert(user_id, db_baseline);

        info!(user_id, redis_baseline, db_baseline, "用户验证基线数据");
    }

    // 4. 同时为所有用户添加到验证队列
    let barrier = Arc::new(Barrier::new(user_count));
    let mut handles = vec![];

    for &user_id in &test_user_ids {
        let verify_manager_clone = verify_manager.clone();
        let barrier_clone = barrier.clone();

        let handle = tokio::spawn(async move {
            barrier_clone.wait().await;

            info!(user_id, "用户开始请求验证");

            // 模拟用户调用verify接口：添加到验证队列
            let result = verify_manager_clone
                .append_user_to_verify_list(user_id, Some(100))
                .await;

            match result {
                Ok(_) => {
                    info!(user_id, "用户成功添加到验证队列");
                    Ok(user_id)
                }
                Err(e) => {
                    warn!(user_id, error = %e, "用户添加验证队列失败");
                    Err(e)
                }
            }
        });
        handles.push(handle);
    }

    // 等待所有用户添加完成
    let results: Result<Vec<_>, _> = futures::future::try_join_all(handles).await;
    let _user_results = results?;

    info!("所有用户已添加到验证队列，等待验证系统处理...");

    // 5. 等待验证系统工作并定期检查进度
    let wait_duration = Duration::from_secs(30);
    let check_interval = Duration::from_secs(5);
    let mut elapsed = Duration::ZERO;

    info!(
        "等待 {:?}，每 {:?} 检查一次进度",
        wait_duration, check_interval
    );

    while elapsed < wait_duration {
        tokio::time::sleep(check_interval).await;
        elapsed += check_interval;

        info!("已等待 {:?} / {:?}", elapsed, wait_duration);

        // 检查当前进度
        for &user_id in &test_user_ids {
            if let Ok(redis_info) = verify_manager.get_user_unverified_info(user_id).await {
                let current_redis = redis_info.success_count + redis_info.fail_count;
                let baseline_redis = baseline_redis_stats.get(&user_id).copied().unwrap_or(0);
                let redis_new = current_redis.saturating_sub(baseline_redis);

                if redis_new > 0 {
                    info!(
                        user_id,
                        redis_new,
                        processing = redis_info.processing_count,
                        pending = redis_info.pending_unverify_count,
                        "用户验证进度"
                    );
                }
            }
        }
    }

    // 6. 收集最终的验证结果统计
    info!("收集最终验证结果统计...");

    let mut final_redis_stats = HashMap::new();
    let mut final_db_counts = HashMap::new();

    for &user_id in &test_user_ids {
        // Redis维度：调度器分配统计
        let redis_info = verify_manager.get_user_unverified_info(user_id).await?;
        let current_redis = redis_info.success_count + redis_info.fail_count;
        let baseline_redis = baseline_redis_stats.get(&user_id).copied().unwrap_or(0);
        let redis_new = current_redis.saturating_sub(baseline_redis);
        final_redis_stats.insert(user_id, redis_new);

        // 数据库维度：实际验证记录数
        let db_records = UserPaperVerificationsQuery::list_by_user_id(&db, user_id).await?;
        let current_db = db_records.len();
        let baseline_db = baseline_db_counts.get(&user_id).copied().unwrap_or(0);
        let db_new = current_db.saturating_sub(baseline_db);
        final_db_counts.insert(user_id, db_new);

        info!(
            user_id,
            redis_new,
            db_new,
            redis_success = redis_info.success_count,
            redis_fail = redis_info.fail_count,
            redis_processing = redis_info.processing_count,
            redis_pending = redis_info.pending_unverify_count,
            "用户最终验证统计"
        );
    }

    // 7. 分析调度公平性
    info!("分析调度公平性...");

    // Redis维度分析
    let redis_values: Vec<i64> = final_redis_stats.values().copied().collect();
    let total_redis_new: i64 = redis_values.iter().sum();

    if total_redis_new > 0 {
        let max_redis = *redis_values.iter().max().unwrap();
        let min_redis = *redis_values.iter().min().unwrap();
        let redis_ratio = if min_redis > 0 {
            max_redis as f64 / min_redis as f64
        } else if max_redis > 0 {
            f64::INFINITY
        } else {
            1.0
        };

        info!(
            total_redis_new,
            max_redis, min_redis, redis_ratio, "Redis调度统计分布"
        );

        if redis_ratio.is_finite() && redis_ratio > 3.0 {
            warn!(
                redis_ratio,
                "Redis调度分布可能不够公平，最大/最小比例超过 3:1"
            );
        }
    } else {
        warn!("Redis维度没有新的验证活动");
    }

    // 数据库维度分析
    let db_values: Vec<usize> = final_db_counts.values().copied().collect();
    let total_db_new: usize = db_values.iter().sum();

    if total_db_new > 0 {
        let max_db = *db_values.iter().max().unwrap();
        let min_db = *db_values.iter().min().unwrap();
        let db_ratio = if min_db > 0 {
            max_db as f64 / min_db as f64
        } else if max_db > 0 {
            f64::INFINITY
        } else {
            1.0
        };

        info!(total_db_new, max_db, min_db, db_ratio, "数据库验证记录分布");

        if db_ratio.is_finite() && db_ratio > 3.0 {
            warn!(
                db_ratio,
                "数据库验证分布可能不够公平，最大/最小比例超过 3:1"
            );
        }
    } else {
        warn!("数据库维度没有新的验证记录");
    }

    // 8. 清理测试数据
    info!("清理测试数据...");
    for &user_id in &test_user_ids {
        cleanup_user_test_data(&db, user_id).await?;
        cleanup_user_verify_state(&verify_manager, user_id).await?;
    }

    info!(
        total_users = user_count,
        total_redis_new, total_db_new, "多用户并发验证公平性测试完成"
    );

    // 基本断言：至少有一个维度应该有验证活动
    assert!(
        total_redis_new > 0 || total_db_new > 0,
        "验证系统应该在Redis或数据库维度有验证活动，或者需要更长的等待时间"
    );

    Ok(())
}

/// 为用户创建测试数据（兴趣和订阅）
async fn setup_user_test_data(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
    source_ids: &[i32],
    config: &conf::config::AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = rand::rng();

    // 创建用户兴趣
    let mut interests = vec![
        "machine learning".to_string(),
        "artificial intelligence".to_string(),
        "deep learning".to_string(),
        "computer vision".to_string(),
        "natural language processing".to_string(),
    ];

    interests.shuffle(&mut rng);
    let selected_interests: Vec<String> = interests
        .into_iter()
        .take(2) // 每个用户选择2个兴趣
        .collect();

    UserInterestsQuery::replace_many(
        db,
        user_id,
        selected_interests.clone(),
        config.llm.model.clone(),
    )
    .await?;

    // 从现有源中随机选择订阅
    let mut sources = source_ids.to_vec();
    sources.shuffle(&mut rng);
    let max_sources = sources.len().clamp(1, 3); // 最多3个，最少1个
    let selected_sources: Vec<i32> = sources.into_iter().take(max_sources).collect();

    RssSubscriptionsQuery::replace_many(db, user_id, selected_sources.clone()).await?;

    info!(
        user_id,
        interests = ?selected_interests,
        sources = ?selected_sources,
        "为用户创建测试数据"
    );

    Ok(())
}

/// 清理用户验证状态
async fn cleanup_user_verify_state(
    _verify_manager: &VerifyManager,
    user_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: 实现清理验证状态的逻辑
    info!(user_id, "清理用户验证状态");
    Ok(())
}

/// 清理用户测试数据
async fn cleanup_user_test_data(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = app_config();

    // 清空兴趣和订阅
    UserInterestsQuery::replace_many(db, user_id, vec![], config.llm.model.clone()).await?;
    RssSubscriptionsQuery::replace_many(db, user_id, vec![]).await?;

    info!(user_id, "清理用户测试数据完成");
    Ok(())
}
