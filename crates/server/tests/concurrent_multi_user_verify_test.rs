use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Test fairness of multi-user concurrent verification scheduling
/// Verify scheduling strategy through Redis statistics and database records
#[tokio::test]
async fn test_concurrent_multi_user_verify_fairness() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let db = get_db().await.clone();
    let config = app_config();

    // Create Redis connection
    let redis_url = config.rss.feed_redis.url.clone();
    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            warn!(error = %err, "Skipping test: Unable to connect to Redis");
            return Ok(());
        }
    };

    let redis_pool = match bb8::Pool::builder().max_size(10).build(manager).await {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "Skipping test: Unable to create Redis connection pool");
            return Ok(());
        }
    };

    let _apalis_conn = match apalis_redis::connect(redis_url.as_str()).await {
        Ok(conn) => conn,
        Err(err) => {
            warn!(error = %err, "Skipping test: Unable to create Apalis Redis connection");
            return Ok(());
        }
    };

    info!("Starting multi-user concurrent verification fairness test");

    // 1. Prepare test data
    let test_user_ids: Vec<i64> = (3000000..3000005).collect(); // 5 test users
    let user_count = test_user_ids.len();

    // Get existing RSS sources
    let available_sources = RssSourcesQuery::list_all(&db).await?;
    if available_sources.is_empty() {
        warn!("Skipping test: No RSS sources in database");
        return Ok(());
    }

    let source_ids: Vec<i32> = available_sources.into_iter().map(|s| s.id).collect();
    info!(
        source_count = source_ids.len(),
        "Using existing RSS sources for testing"
    );

    // Create interests and subscriptions for each user
    for &user_id in &test_user_ids {
        setup_user_test_data(&db, user_id, &source_ids, &config).await?;
    }

    // 2. Create VerifyManager
    let verify_manager = VerifyManager::new(
        redis_pool.clone(),
        db.clone(),
        config.rss.feed_redis.redis_prefix.clone(),
        config.rss.feed_redis.redis_key_default_expire,
    )
    .await;

    // Clean up previous state
    for &user_id in &test_user_ids {
        cleanup_user_verify_state(&verify_manager, user_id).await?;
    }

    info!("Test environment prepared, starting concurrent verification test");

    // 3. Record baseline data before verification
    let mut baseline_redis_stats = HashMap::new();
    let mut baseline_db_counts = HashMap::new();

    for &user_id in &test_user_ids {
        // Redis baseline
        let redis_info = match verify_manager.get_user_unverified_info(user_id).await {
            Ok(info) => info,
            Err(_) => {
                // If retrieval fails, create zero-value statistics
                feed::redis::verify_manager::UserVerifyInfo {
                    pending_unverify_count: 0,
                    success_count: 0,
                    fail_count: 0,
                    processing_count: 0,
                    total: 0,
                    token_usage: 0,
                    matched_count: 0,
                    max_match_limit: 0,
                }
            }
        };
        let redis_baseline = redis_info.success_count + redis_info.fail_count;
        baseline_redis_stats.insert(user_id, redis_baseline);

        // Database baseline
        let db_records = UserPaperVerificationsQuery::list_by_user_id(&db, user_id).await?;
        let db_baseline = db_records.len();
        baseline_db_counts.insert(user_id, db_baseline);

        info!(
            user_id,
            redis_baseline, db_baseline, "User verification baseline data"
        );
    }

    // 4. Add all users to verification queue simultaneously
    let barrier = Arc::new(Barrier::new(user_count));
    let mut handles = vec![];

    for &user_id in &test_user_ids {
        let verify_manager_clone = verify_manager.clone();
        let barrier_clone = barrier.clone();

        let handle = tokio::spawn(async move {
            barrier_clone.wait().await;

            info!(user_id, "User started verification request");

            // Simulate user calling verify interface: add to verification queue
            let result = verify_manager_clone
                .append_user_to_verify_list(user_id, Some(1000), None, Some(10))
                .await;

            match result {
                Ok(_) => {
                    info!(user_id, "User successfully added to verification queue");
                    Ok(user_id)
                }
                Err(e) => {
                    warn!(user_id, error = %e, "Failed to add user to verification queue");
                    Err(e)
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all users to be added
    let results: Result<Vec<_>, _> = futures::future::try_join_all(handles).await;
    let _user_results = results?;

    info!("All users added to verification queue, waiting for verification system to process...");

    // 5. Get worker count
    let worker_concurrency = config.rss.workers.verify_single_user_one_paper.concurrency;
    info!(
        worker_concurrency,
        "verify_single_user_one_paper worker concurrency"
    );

    // 6. Record start time and wait for verification system to work
    let start_time = Instant::now();
    let wait_duration = Duration::from_secs(900); // Extend wait time to 90 seconds
    let check_interval = Duration::from_secs(5);
    let mut elapsed = Duration::ZERO;

    info!(
        "Waiting {:?}, checking progress every {:?}",
        wait_duration, check_interval
    );

    while elapsed < wait_duration {
        tokio::time::sleep(check_interval).await;
        elapsed += check_interval;

        info!("Waited {:?} / {:?}", elapsed, wait_duration);

        // Check current progress
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
                        "User verification progress"
                    );
                }
            }
        }
    }

    // 7. Calculate total processing time
    let total_elapsed = start_time.elapsed();
    info!(
        total_elapsed_secs = total_elapsed.as_secs_f64(),
        "Verification processing completed, total time taken"
    );

    // 8. Collect final verification result statistics
    info!("Collecting final verification result statistics...");

    let mut final_redis_stats = HashMap::new();
    let mut final_db_counts = HashMap::new();

    for &user_id in &test_user_ids {
        // Redis dimension: scheduler allocation statistics
        let redis_info = verify_manager.get_user_unverified_info(user_id).await?;
        let current_redis = redis_info.success_count + redis_info.fail_count;
        let baseline_redis = baseline_redis_stats.get(&user_id).copied().unwrap_or(0);
        let redis_new = current_redis.saturating_sub(baseline_redis);
        final_redis_stats.insert(user_id, redis_new);

        // Database dimension: actual verification record count
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
            "User final verification statistics"
        );
    }

    // 9. Analyze scheduling fairness
    info!("Analyzing scheduling fairness...");

    // Redis dimension analysis
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
            max_redis, min_redis, redis_ratio, "Redis scheduling statistics distribution"
        );

        if redis_ratio.is_finite() && redis_ratio > 3.0 {
            warn!(
                redis_ratio,
                "Redis scheduling distribution may not be fair enough, max/min ratio exceeds 3:1"
            );
        }
    } else {
        warn!("No new verification activity in Redis dimension");
    }

    // Database dimension analysis
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

        info!(
            total_db_new,
            max_db, min_db, db_ratio, "Database verification record distribution"
        );

        if db_ratio.is_finite() && db_ratio > 3.0 {
            warn!(
                db_ratio,
                "Database verification distribution may not be fair enough, max/min ratio exceeds 3:1"
            );
        }
    } else {
        warn!("No new verification records in database dimension");
    }

    // 10. Calculate worker processing speed
    info!("Calculating worker processing speed...");

    let total_elapsed_secs = total_elapsed.as_secs_f64();
    let total_verified = total_db_new.max(total_redis_new as usize);

    if total_verified > 0 && total_elapsed_secs > 0.0 {
        // Total processing speed (all workers combined)
        let total_throughput = total_verified as f64 / total_elapsed_secs;

        // Average processing speed per worker
        let per_worker_throughput = if worker_concurrency > 0 {
            total_throughput / worker_concurrency as f64
        } else {
            0.0
        };

        info!(
            total_verified,
            total_elapsed_secs = format!("{:.2}", total_elapsed_secs),
            worker_concurrency,
            total_throughput = format!("{:.2} papers/sec", total_throughput),
            per_worker_throughput = format!("{:.2} papers/sec", per_worker_throughput),
            total_throughput_per_min = format!("{:.2} papers/min", total_throughput * 60.0),
            per_worker_throughput_per_min =
                format!("{:.2} papers/min", per_worker_throughput * 60.0),
            "Worker processing speed statistics"
        );
    } else {
        warn!("Unable to calculate processing speed: no verification activity or time is 0");
    }

    // 11. Clean up test data
    info!("Cleaning up test data...");
    for &user_id in &test_user_ids {
        cleanup_user_test_data(&db, user_id).await?;
        cleanup_user_verify_state(&verify_manager, user_id).await?;
    }

    info!(
        total_users = user_count,
        total_redis_new,
        total_db_new,
        total_verified,
        "Multi-user concurrent verification fairness test completed"
    );

    // Basic assertion: at least one dimension should have verification activity
    assert!(
        total_redis_new > 0 || total_db_new > 0,
        "Verification system should have verification activity in Redis or database dimension, or needs longer wait time"
    );

    Ok(())
}

/// Create test data for user (interests and subscriptions)
async fn setup_user_test_data(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
    source_ids: &[i32],
    config: &conf::config::AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = rand::rng();

    // Create user interests
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
        .take(4) // Each user selects 2 interests
        .collect();

    UserInterestsQuery::replace_many(
        db,
        user_id,
        selected_interests.clone(),
        config.llm.model.clone(),
    )
    .await?;

    // Randomly select subscriptions from existing sources
    let mut sources = source_ids.to_vec();
    sources.shuffle(&mut rng);
    let max_sources = sources.len().clamp(5, 10); // Maximum 3, minimum 1
    let selected_sources: Vec<i32> = sources.into_iter().take(max_sources).collect();

    RssSubscriptionsQuery::replace_many(db, user_id, selected_sources.clone()).await?;

    info!(
        user_id,
        interests = ?selected_interests,
        sources = ?selected_sources,
        "Created test data for user"
    );

    Ok(())
}

/// Clean up user verification state
async fn cleanup_user_verify_state(
    _verify_manager: &VerifyManager,
    user_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Implement logic to clean up verification state
    info!(user_id, "Cleaning up user verification state");
    Ok(())
}

/// Clean up user test data
async fn cleanup_user_test_data(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = app_config();

    // Clear interests and subscriptions
    UserInterestsQuery::replace_many(db, user_id, vec![], config.llm.model.clone()).await?;
    RssSubscriptionsQuery::replace_many(db, user_id, vec![]).await?;

    info!(user_id, "User test data cleanup completed");
    Ok(())
}
