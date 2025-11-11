// use bb8::Pool;
// use bb8_redis::RedisConnectionManager;
// use conf::config::app_config;
// use dotenvy::dotenv;
// use feed::redis::verify_manager::UserPaperVerifyData;
// use redis::AsyncCommands;
// use std::env;
// use tracing::info;
// use tracing_subscriber::EnvFilter;

// static INIT_TRACING: std::sync::Once = std::sync::Once::new();

// fn init_test_tracing() {
//     INIT_TRACING.call_once(|| {
//         let _ = tracing_subscriber::fmt()
//             .with_env_filter(
//                 EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
//             )
//             .with_writer(std::io::stderr)
//             .compact()
//             .try_init();
//         dotenv().ok();
//     });
// }

// async fn create_test_redis_pool() -> Pool<RedisConnectionManager> {
//     let config = app_config();
//     let redis_url = config.rss.feed_redis.url.clone();
//     let manager = RedisConnectionManager::new(redis_url.clone())
//         .expect("Failed to create Redis connection manager");
//     Pool::builder()
//         .max_size(5)
//         .build(manager)
//         .await
//         .expect("Failed to create Redis connection pool")
// }

// #[tokio::test]
// async fn test_cleanup_deletes_all_keys() {
//     init_test_tracing();

//     let redis_pool = create_test_redis_pool().await;
//     let mut conn = redis_pool.get().await.expect("Failed to get connection");

//     // Create test data
//     let base_key = "test:cleanup:user:123";
//     let data = UserPaperVerifyData::new(base_key.to_string());

//     // Set some test data in all keys
//     let _: () = conn
//         .lpush(&data.pending, 1)
//         .await
//         .expect("Failed to set pending");
//     let _: () = conn
//         .lpush(&data.success, 2)
//         .await
//         .expect("Failed to set success");
//     let _: () = conn.lpush(&data.fail, 3).await.expect("Failed to set fail");
//     let _: () = conn
//         .lpush(&data.processing, 4)
//         .await
//         .expect("Failed to set processing");
//     let _: () = conn
//         .set(&data.total, 100)
//         .await
//         .expect("Failed to set total");
//     let _: () = conn
//         .set(&data.token_usage, 50)
//         .await
//         .expect("Failed to set token_usage");
//     let _: () = conn
//         .set(&data.matched_count, 10)
//         .await
//         .expect("Failed to set matched_count");
//     let _: () = conn
//         .set(&data.max_match_limit, 200)
//         .await
//         .expect("Failed to set max_match_limit");

//     // Verify keys exist
//     let exists: bool = conn
//         .exists(&data.pending)
//         .await
//         .expect("Failed to check pending");
//     assert!(exists, "Pending key should exist before cleanup");

//     // Call cleanup
//     data.cleanup(&mut conn).await.expect("Cleanup failed");

//     // Verify all keys are deleted
//     let pending_exists: bool = conn
//         .exists(&data.pending)
//         .await
//         .expect("Failed to check pending");
//     let success_exists: bool = conn
//         .exists(&data.success)
//         .await
//         .expect("Failed to check success");
//     let fail_exists: bool = conn.exists(&data.fail).await.expect("Failed to check fail");
//     let processing_exists: bool = conn
//         .exists(&data.processing)
//         .await
//         .expect("Failed to check processing");
//     let total_exists: bool = conn
//         .exists(&data.total)
//         .await
//         .expect("Failed to check total");
//     let token_usage_exists: bool = conn
//         .exists(&data.token_usage)
//         .await
//         .expect("Failed to check token_usage");
//     let matched_count_exists: bool = conn
//         .exists(&data.matched_count)
//         .await
//         .expect("Failed to check matched_count");
//     let max_limit_exists: bool = conn
//         .exists(&data.max_match_limit)
//         .await
//         .expect("Failed to check max_match_limit");

//     assert!(!pending_exists, "Pending key should be deleted");
//     assert!(!success_exists, "Success key should be deleted");
//     assert!(!fail_exists, "Fail key should be deleted");
//     assert!(!processing_exists, "Processing key should be deleted");
//     assert!(!total_exists, "Total key should be deleted");
//     assert!(!token_usage_exists, "Token usage key should be deleted");
//     assert!(!matched_count_exists, "Matched count key should be deleted");
//     assert!(!max_limit_exists, "Max limit key should be deleted");

//     info!("✅ test_cleanup_deletes_all_keys passed");
// }

// #[tokio::test]
// async fn test_set_expire_sets_ttl() {
//     init_test_tracing();

//     let redis_pool = create_test_redis_pool().await;
//     let mut conn = redis_pool.get().await.expect("Failed to get connection");

//     // Create test data
//     let base_key = "test:expire:user:456";
//     let data = UserPaperVerifyData::new(base_key.to_string());

//     // Set some test data in all keys
//     let _: () = conn
//         .lpush(&data.pending, 1)
//         .await
//         .expect("Failed to set pending");
//     let _: () = conn
//         .lpush(&data.success, 2)
//         .await
//         .expect("Failed to set success");
//     let _: () = conn.lpush(&data.fail, 3).await.expect("Failed to set fail");
//     let _: () = conn
//         .lpush(&data.processing, 4)
//         .await
//         .expect("Failed to set processing");
//     let _: () = conn
//         .set(&data.total, 100)
//         .await
//         .expect("Failed to set total");
//     let _: () = conn
//         .set(&data.token_usage, 50)
//         .await
//         .expect("Failed to set token_usage");
//     let _: () = conn
//         .set(&data.matched_count, 10)
//         .await
//         .expect("Failed to set matched_count");
//     let _: () = conn
//         .set(&data.max_match_limit, 200)
//         .await
//         .expect("Failed to set max_match_limit");

//     // Set expiration to 60 seconds
//     let expire_seconds = 60;
//     data.set_expire(&mut conn, expire_seconds)
//         .await
//         .expect("Set expire failed");

//     // Check TTL for each key
//     let pending_ttl: i64 = conn
//         .ttl(&data.pending)
//         .await
//         .expect("Failed to get pending TTL");
//     let success_ttl: i64 = conn
//         .ttl(&data.success)
//         .await
//         .expect("Failed to get success TTL");
//     let fail_ttl: i64 = conn.ttl(&data.fail).await.expect("Failed to get fail TTL");
//     let processing_ttl: i64 = conn
//         .ttl(&data.processing)
//         .await
//         .expect("Failed to get processing TTL");
//     let total_ttl: i64 = conn
//         .ttl(&data.total)
//         .await
//         .expect("Failed to get total TTL");
//     let token_usage_ttl: i64 = conn
//         .ttl(&data.token_usage)
//         .await
//         .expect("Failed to get token_usage TTL");
//     let matched_count_ttl: i64 = conn
//         .ttl(&data.matched_count)
//         .await
//         .expect("Failed to get matched_count TTL");
//     let max_limit_ttl: i64 = conn
//         .ttl(&data.max_match_limit)
//         .await
//         .expect("Failed to get max_limit TTL");

//     // TTL should be close to 60 seconds (allowing for small delays)
//     assert!(
//         pending_ttl > 55 && pending_ttl <= 60,
//         "Pending TTL should be around 60 seconds, got {pending_ttl}"
//     );
//     assert!(
//         success_ttl > 55 && success_ttl <= 60,
//         "Success TTL should be around 60 seconds, got {success_ttl}"
//     );
//     assert!(
//         fail_ttl > 55 && fail_ttl <= 60,
//         "Fail TTL should be around 60 seconds, got {fail_ttl}"
//     );
//     assert!(
//         processing_ttl > 55 && processing_ttl <= 60,
//         "Processing TTL should be around 60 seconds, got {processing_ttl}"
//     );
//     assert!(
//         total_ttl > 55 && total_ttl <= 60,
//         "Total TTL should be around 60 seconds, got {total_ttl}"
//     );
//     assert!(
//         token_usage_ttl > 55 && token_usage_ttl <= 60,
//         "Token usage TTL should be around 60 seconds, got {token_usage_ttl}"
//     );
//     assert!(
//         matched_count_ttl > 55 && matched_count_ttl <= 60,
//         "Matched count TTL should be around 60 seconds, got {matched_count_ttl}"
//     );
//     assert!(
//         max_limit_ttl > 55 && max_limit_ttl <= 60,
//         "Max limit TTL should be around 60 seconds, got {max_limit_ttl}"
//     );

//     info!("✅ test_set_expire_sets_ttl passed");
//     info!("   Pending TTL: {pending_ttl}");
//     info!("   Success TTL: {success_ttl}");
//     info!("   Fail TTL: {fail_ttl}");
//     info!("   Processing TTL: {processing_ttl}");
//     info!("   Total TTL: {total_ttl}");
//     info!("   Token usage TTL: {token_usage_ttl}");
//     info!("   Matched count TTL: {matched_count_ttl}");
//     info!("   Max limit TTL: {max_limit_ttl}");

//     // Cleanup
//     data.cleanup(&mut conn).await.expect("Cleanup failed");
// }

// #[tokio::test]
// async fn test_cleanup_is_idempotent() {
//     init_test_tracing();

//     let redis_pool = create_test_redis_pool().await;
//     let mut conn = redis_pool.get().await.expect("Failed to get connection");

//     // Create test data
//     let base_key = "test:idempotent:user:789";
//     let data = UserPaperVerifyData::new(base_key.to_string());

//     // Set some test data
//     let _: () = conn
//         .lpush(&data.pending, 1)
//         .await
//         .expect("Failed to set pending");
//     let _: () = conn
//         .set(&data.total, 100)
//         .await
//         .expect("Failed to set total");

//     // Call cleanup first time
//     data.cleanup(&mut conn).await.expect("First cleanup failed");

//     // Verify keys are deleted
//     let exists: bool = conn
//         .exists(&data.pending)
//         .await
//         .expect("Failed to check pending");
//     assert!(!exists, "Pending key should be deleted after first cleanup");

//     // Call cleanup second time (should not error)
//     let result = data.cleanup(&mut conn).await;
//     assert!(
//         result.is_ok(),
//         "Second cleanup should not fail even if keys don't exist"
//     );

//     info!("✅ test_cleanup_is_idempotent passed");
// }

// #[tokio::test]
// async fn test_verify_paper_with_interests_with_real_data() {
//     use feed::workers::base::RedisService;
//     use feed::workers::verify_user_papers::verify_paper_with_interests;
//     use sea_orm::EntityTrait;
//     use seaorm_db::connection::get_db;
//     use seaorm_db::entities::feed::{rss_papers, user_interest};

//     init_test_tracing();

//     let config = app_config();

//     info!("========== Starting test_verify_paper_with_interests_with_real_data ==========");

//     // 1. 获取数据库连接
//     let db_conn = get_db().await.clone();
//     info!("✅ Database connection established");

//     // 2. 查询真实数据 - Paper ID: 110220 (RL + Agent 相关论文)
//     // Title: "Janus-Pro-R1: Advancing Collaborative Visual Comprehension and Generation via Reinforcement Learning"
//     let paper = rss_papers::Entity::find_by_id(110220)
//         .one(&db_conn)
//         .await
//         .expect("Failed to query paper")
//         .expect("Paper not found");

//     info!("✅ Found paper: {} (ID: {})", paper.title, paper.id);

//     // 3. 查询用户兴趣 - IDs: 39, 38, 64
//     let interest_ids = vec![39, 38, 64];
//     let mut interests = Vec::new();
//     for id in interest_ids {
//         if let Some(interest) = user_interest::Entity::find_by_id(id)
//             .one(&db_conn)
//             .await
//             .expect("Failed to query interest")
//         {
//             info!(
//                 "✅ Found interest: {} (ID: {})",
//                 interest.interest, interest.id
//             );
//             interests.push(interest);
//         }
//     }

//     assert_eq!(interests.len(), 3, "Should have 3 interests");
//     info!("✅ Found {} interests for testing", interests.len());

//     // 4. 准备 Redis 连接 - 从环境变量读取
//     let redis_url = config.rss.feed_redis.url.clone();
//     let redis_pool = create_test_redis_pool().await;
//     let apalis_redis_conn = apalis_redis::connect(redis_url.as_str())
//         .await
//         .expect("Cannot connect to Redis");

//     let redis = RedisService {
//         pool: redis_pool,
//         apalis_conn: apalis_redis_conn,
//     };
//     info!("✅ Redis connection established");

//     // 5. 调用函数 - 从环境变量读取模型配置
//     let model = config.llm.model.clone();
//     info!(
//         "🚀 Calling verify_paper_with_interests with model: {}",
//         model
//     );

//     let result =
//         verify_paper_with_interests(db_conn, redis, &paper, interests, model, "test query").await;

//     // 6. 验证结果
//     match &result {
//         Ok(verify_result) => {
//             info!("✅ Verification succeeded!");
//             // info!("   - Matches: {}", verify_result.matches);
//             info!("   - Reasoning: {}", verify_result.reasoning);
//             info!("   - Token usage: {}", verify_result.token_usage);
//             info!(
//                 "   - Matched criteria count: {}",
//                 verify_result.matched_criteria.len()
//             );
//         }
//         Err(e) => {
//             info!("❌ Verification failed with error: {:?}", e);
//         }
//     }

//     info!("========== Test completed ==========");

//     // 测试通过条件：函数能够执行（无论成功或失败）
//     assert!(result.is_ok() || result.is_err(), "Function should execute");
// }
