use std::sync::Arc;

use conf::config::app_config;
use dotenvy::dotenv;
use seaorm_db::connection::get_db;
use seaorm_db::query::feed::user_interests::UserInterestsQuery;
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

/// 测试同一用户并发调用 replace_many 方法
/// 这应该测试数据一致性，确保最终结果正确
#[tokio::test]
async fn test_same_user_concurrent_replace_many() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let db = get_db().await.clone();
    let config = app_config();
    let test_user_id = 999999; // 使用一个测试用户ID

    info!("开始测试同一用户并发调用 replace_many");

    // 清理测试数据
    cleanup_test_data(&db, test_user_id).await?;

    let barrier = Arc::new(Barrier::new(3)); // 3个并发任务
    let mut handles = vec![];

    // 创建3个并发任务，每个任务设置不同的兴趣列表
    for i in 0..3 {
        let db_clone = db.clone();
        let config_clone = config.clone();
        let barrier_clone = barrier.clone();

        let handle = tokio::spawn(async move {
            let interests = vec![
                format!("interest_1"),
                format!("interest_2"),
                format!("interest_3"),
            ];

            info!(task_id = i, ?interests, "任务 {} 准备设置兴趣", i);

            // 等待所有任务同时开始
            barrier_clone.wait().await;

            info!(task_id = i, "任务 {} 开始执行 replace_many", i);
            let start_time = std::time::Instant::now();

            let result = UserInterestsQuery::replace_many(
                &db_clone,
                test_user_id,
                interests.clone(),
                config_clone.llm.model.clone(),
            )
            .await;

            let duration = start_time.elapsed();
            info!(
                task_id = i,
                duration_ms = duration.as_millis(),
                "任务 {} 完成",
                i
            );

            match result {
                Ok(ids) => {
                    info!(task_id = i, ?ids, "任务 {} 成功，返回ID列表", i);

                    // 输出当前兴趣列表
                    let current_interests =
                        UserInterestsQuery::list_by_user_id(&db_clone, test_user_id).await?;
                    info!(
                        task_id = i,
                        ?current_interests,
                        "任务 {} 执行后的当前兴趣列表",
                        i
                    );

                    Ok(ids)
                }
                Err(e) => {
                    warn!(task_id = i, error = %e, "任务 {} 失败", i);
                    Err(e)
                }
            }
        });

        handles.push(handle);
    }

    // 等待所有任务完成
    let results: Result<Vec<_>, _> = futures::future::try_join_all(handles).await;
    let _task_results = results?;

    info!("所有并发任务完成，开始验证结果");

    // 验证最终结果
    let final_interests = UserInterestsQuery::list_by_user_id(&db, test_user_id).await?;
    info!(
        count = final_interests.len(),
        ?final_interests,
        "最终兴趣列表"
    );

    // 验证数据一致性
    assert_eq!(final_interests.len(), 3, "最终应该有3个兴趣");

    // 验证兴趣名称的唯一性
    let interest_names: Vec<String> = final_interests.into_iter().map(|m| m.interest).collect();
    let unique_names: std::collections::HashSet<String> = interest_names.iter().cloned().collect();
    assert_eq!(unique_names.len(), 3, "所有兴趣名称应该是唯一的");

    // 清理测试数据
    cleanup_test_data(&db, test_user_id).await?;

    info!("同一用户并发测试完成");
    Ok(())
}

/// 清理测试数据
async fn cleanup_test_data(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // 输出清理前的兴趣列表
    let before_interests = UserInterestsQuery::list_by_user_id(db, user_id).await?;
    info!(
        user_id,
        ?before_interests,
        "清理前用户 {} 的兴趣列表",
        user_id
    );

    // 使用 replace_many 清空用户的所有兴趣
    let config = app_config();
    UserInterestsQuery::replace_many(db, user_id, vec![], config.llm.model.clone()).await?;

    // 输出清理后的兴趣列表
    let after_interests = UserInterestsQuery::list_by_user_id(db, user_id).await?;
    info!(
        user_id,
        ?after_interests,
        "清理后用户 {} 的兴趣列表",
        user_id
    );

    info!(user_id, "清理用户 {} 的测试数据完成", user_id);
    Ok(())
}
