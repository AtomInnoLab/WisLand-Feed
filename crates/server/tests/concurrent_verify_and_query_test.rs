use std::sync::Arc;
use std::time::Duration;

use conf::config::app_config;
use dotenvy::dotenv;
use feed::redis::pubsub::RedisPubSubManager;
use feed::redis::verify_manager::VerifyManager;
use seaorm_db::connection::get_db;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use axum::extract::{Query, State};
use server::middlewares::auth::{User, UserInfo};
use server::routers::feed::feeds::{AllVerifiedPapersRequest, all_verified_papers};
use server::routers::feed::paper::{PapersRequest, unverified_papers};
use server::state::app_state::{AppState, RedisService};

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

/// Test concurrent calls to append_user_to_verify_list and unverified_papers
/// Verify data consistency during concurrent verification and query operations
#[tokio::test]
async fn test_concurrent_append_and_query() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let test_user_id = 1i64;
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

    // Create PubSubManager
    let pubsub_manager = RedisPubSubManager::new(redis_url.as_str()).await;

    // Create Apalis connection
    let apalis_conn = match apalis_redis::connect(redis_url.as_str()).await {
        Ok(conn) => conn,
        Err(err) => {
            warn!(error = %err, "Skipping test: Unable to create Apalis Redis connection");
            return Ok(());
        }
    };

    // Create AppState
    let state = Arc::new(AppState {
        conn: db.clone(),
        config: config.clone(),
        redis: RedisService {
            pool: redis_pool.clone(),
            apalis_conn,
            pubsub_manager,
        },
    });

    // Create VerifyManager
    let verify_manager = VerifyManager::new(
        redis_pool.clone(),
        db.clone(),
        config.rss.feed_redis.redis_prefix.clone(),
        config.rss.feed_redis.redis_key_default_expire,
    )
    .await;

    let rounds = 10; // 10 rounds of testing
    let append_count = 3; // 3 append calls per round
    let query_count = 6; // 6 query calls per round

    for round in 0..rounds {
        info!("=== Round {} ===", round + 1);

        // Clean up previous round's state
        let _ = verify_manager.finish_user_verify(test_user_id, false).await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut all_handles = vec![];

        // 2. Start multiple append_user_to_verify_list calls simultaneously
        for i in 0..append_count {
            let vm = verify_manager.clone();
            let handle = tokio::spawn(async move {
                // 添加小延迟，模拟真实场景中的时间差异
                tokio::time::sleep(Duration::from_millis(i as u64 * 10)).await;

                info!("Starting append_user_to_verify_list #{}", i);
                let result = vm
                    .append_user_to_verify_list(
                        test_user_id,
                        Some(1000), // limit
                        None,       // channel
                        10,         // max_match_limit
                    )
                    .await;

                ("append", i, result)
            });
            all_handles.push(handle);
        }

        // 等待 append 任务启动后再开始 query 任务
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 3. Start multiple unverified_papers queries simultaneously
        let mut query_handles = vec![];
        for i in 0..query_count {
            let state_clone = state.clone();
            let handle = tokio::spawn(async move {
                let user_info = UserInfo {
                    id: test_user_id,
                    open_id: "test_user".to_string(),
                    name: Some("test_user".to_string()),
                    given_name: None,
                    family_name: None,
                    nickname: None,
                    preferred_username: None,
                    profile: None,
                    picture: None,
                    website: None,
                    email: None,
                    email_verified: None,
                    gender: None,
                    birthdate: None,
                    zoneinfo: None,
                    locale: None,
                    phone_number: None,
                    phone_number_verified: None,
                    address: None,
                };
                let user = User(user_info);
                let payload = PapersRequest {
                    page: Some(1),
                    page_size: Some(20),
                    channel: None,
                    keyword: None,
                    rss_source_id: None,
                    not_match: None,
                };

                // 添加小延迟，模拟真实场景中的时间差异
                tokio::time::sleep(Duration::from_millis(i as u64 * 15)).await;

                info!("Starting unverified_papers #{}", i);
                let result =
                    unverified_papers(State((*state_clone).clone()), user, Query(payload)).await;

                (i, result)
            });
            query_handles.push(handle);
        }

        // 4. Start multiple all_verified_papers queries simultaneously
        let verified_count = 6; // 每轮6个 all_verified_papers 查询
        let mut verified_handles = vec![];
        for i in 0..verified_count {
            let state_clone = state.clone();
            let handle = tokio::spawn(async move {
                let user_info = UserInfo {
                    id: test_user_id,
                    open_id: "test_user".to_string(),
                    name: Some("test_user".to_string()),
                    given_name: None,
                    family_name: None,
                    nickname: None,
                    preferred_username: None,
                    profile: None,
                    picture: None,
                    website: None,
                    email: None,
                    email_verified: None,
                    gender: None,
                    birthdate: None,
                    zoneinfo: None,
                    locale: None,
                    phone_number: None,
                    phone_number_verified: None,
                    address: None,
                };
                let user = User(user_info);

                let payload =
                    serde_json::from_value::<AllVerifiedPapersRequest>(serde_json::json!({
                        "page": 1,
                        "page_size": 20,
                        "channel": null,
                        "matches": null,
                        "user_interest_ids": null,
                        "time_range": null,
                        "ignore_time_range": true,
                        "keyword": null,
                        "rss_source_id": null
                    }))
                    .unwrap();

                // 添加小延迟，模拟真实场景中的时间差异
                tokio::time::sleep(Duration::from_millis(i as u64 * 20)).await;

                info!("Starting all_verified_papers #{}", i);
                let result =
                    all_verified_papers(State((*state_clone).clone()), user, Query(payload)).await;

                (i, result)
            });
            verified_handles.push(handle);
        }

        // 5. Wait for all tasks to complete
        let _append_results = futures::future::join_all(all_handles).await;
        let query_results = futures::future::join_all(query_handles).await;
        let verified_results = futures::future::join_all(verified_handles).await;

        // 5. Collect and verify unverified_papers results
        let mut totals = vec![];
        let mut lengths = vec![];
        let mut paper_ids_sets = vec![];

        for (req_num, response) in query_results.into_iter().flatten() {
            if let Ok(response) = response {
                let data = response.data;
                totals.push(data.pagination.total);
                lengths.push(data.papers.len());

                let ids: Vec<i32> = data
                    .papers
                    .iter()
                    .filter_map(|p| p.id.map(|id| id as i32))
                    .collect();
                paper_ids_sets.push(ids);

                info!(
                    "Round {} - unverified_papers #{}: total={}, papers_len={}",
                    round + 1,
                    req_num,
                    data.pagination.total,
                    data.papers.len()
                );
            }
        }

        // 6. Collect and verify all_verified_papers results
        let mut verified_lengths = vec![];

        for (req_num, response) in verified_results.into_iter().flatten() {
            if let Ok(response) = response {
                let data = response.data;
                verified_lengths.push(data.papers.len());

                info!(
                    "Round {} - all_verified_papers #{}: papers_len={}",
                    round + 1,
                    req_num,
                    data.papers.len()
                );
            }
        }

        // 7. Verify all_verified_papers consistency
        if verified_lengths.len() >= 2 {
            let first_len = verified_lengths[0];
            let all_lengths_equal = verified_lengths.iter().all(|&l| l == first_len);

            info!(
                "Round {}: all_verified_papers papers.len() values: {:?}, consistent: {}",
                round + 1,
                verified_lengths,
                all_lengths_equal
            );

            assert!(
                all_lengths_equal,
                "Round {}: all_verified_papers papers.len() not consistent: {:?}",
                round + 1,
                verified_lengths
            );
        }

        // 8. Verify unverified_papers consistency
        if totals.len() >= 2 {
            let first_total = totals[0];
            let all_totals_equal = totals.iter().all(|&t| t == first_total);

            info!(
                "Round {}: pagination.total values: {:?}, consistent: {}",
                round + 1,
                totals,
                all_totals_equal
            );

            let first_len = lengths[0];
            let all_lengths_equal = lengths.iter().all(|&l| l == first_len);

            info!(
                "Round {}: papers.len() values: {:?}, consistent: {}",
                round + 1,
                lengths,
                all_lengths_equal
            );

            // Verify paper IDs
            let first_ids = &paper_ids_sets[0];
            let mut all_ids_equal = true;
            for (i, ids) in paper_ids_sets.iter().enumerate().skip(1) {
                if ids != first_ids {
                    warn!(
                        "Round {}: unverified_papers #{} paper IDs differ from #0",
                        round + 1,
                        i
                    );
                    all_ids_equal = false;
                }
            }

            info!(
                "Round {}: paper IDs consistent: {}",
                round + 1,
                all_ids_equal
            );

            // Assertions: total and length should be consistent
            assert!(
                all_totals_equal,
                "Round {}: pagination.total not consistent: {:?}",
                round + 1,
                totals
            );

            assert!(
                all_lengths_equal,
                "Round {}: papers.len() not consistent: {:?}",
                round + 1,
                lengths
            );

            assert!(
                all_ids_equal,
                "Round {}: paper IDs not consistent",
                round + 1
            );
        }

        // 等待一段时间让验证过程有时间进行，并模拟真实场景
        tokio::time::sleep(Duration::from_millis(800)).await;
    }

    // 7. Cleanup
    let _ = verify_manager.finish_user_verify(test_user_id, false).await;
    info!("Test completed successfully!");

    Ok(())
}
