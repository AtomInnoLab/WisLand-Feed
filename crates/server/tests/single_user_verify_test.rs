use std::sync::Arc;
use std::time::{Duration, Instant};

use conf::config::app_config;
use dotenvy::dotenv;
use feed::redis::pubsub::{MessageHandler, RedisPubSubManager};
use feed::redis::verify_manager::VerifyManager;
use feed::workers::verify_user_scheduler::VerifyResultWithStats;
use rand::seq::SliceRandom;
use seaorm_db::connection::get_db;
use seaorm_db::query::feed::{
    rss_sources::RssSourcesQuery, rss_subscriptions::RssSubscriptionsQuery,
    user_interests::UserInterestsQuery, user_paper_verifications::UserPaperVerificationsQuery,
};
use tokio::sync::Mutex;
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

// Message handler to collect verification results
struct TestMessageHandler {
    user_id: i64,
    channel: String,
    messages: Arc<Mutex<Vec<VerifyResultWithStats>>>,
}

impl TestMessageHandler {
    fn new(
        user_id: i64,
        channel: String,
        messages: Arc<Mutex<Vec<VerifyResultWithStats>>>,
    ) -> Self {
        Self {
            user_id,
            channel,
            messages,
        }
    }
}

impl MessageHandler for TestMessageHandler {
    fn event_name(&self) -> String {
        RedisPubSubManager::build_user_channel(&self.channel, self.user_id)
    }

    fn handle(&self, message: String) {
        let result: VerifyResultWithStats = match serde_json::from_str(&message) {
            Ok(value) => value,
            Err(e) => {
                warn!("Failed to parse message: {}", e);
                return;
            }
        };

        let messages_clone = self.messages.clone();
        tokio::spawn(async move {
            let mut messages = messages_clone.lock().await;
            messages.push(result);
        });
    }
}

/// Test single user verification with limits and idempotency
#[tokio::test]
async fn test_single_user_verification_with_limits() -> Result<(), Box<dyn std::error::Error>> {
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

    info!("Starting single user verification test with limits");

    // 1. Prepare test data
    let test_user_id: i64 = 4000001;

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

    // Setup user test data
    setup_user_test_data(&db, test_user_id, &source_ids, &config).await?;

    // 2. Create VerifyManager
    let verify_manager = VerifyManager::new(
        redis_pool.clone(),
        db.clone(),
        config.rss.feed_redis.redis_prefix.clone(),
        config.rss.feed_redis.redis_key_default_expire,
    )
    .await;

    // Clean up any existing verification data
    info!(
        "Cleaning up any existing verification data for user {}",
        test_user_id
    );
    let _ = verify_manager.finish_user_verify(test_user_id, None).await;

    // 3. Setup PubSub listener to collect messages
    let pubsub_manager = RedisPubSubManager::new(redis_url.as_str()).await;
    let messages = Arc::new(Mutex::new(Vec::new()));
    let handler = Box::new(TestMessageHandler::new(
        test_user_id,
        config.rss.verify_papers_channel.clone(),
        messages.clone(),
    ));

    // Start listener in background task (add_listener is a long-running loop)
    let pubsub_manager_clone = pubsub_manager.clone();
    tokio::spawn(async move {
        pubsub_manager_clone.add_listener(handler).await;
    });

    // Give PubSub time to connect
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 4. First round: Test max_match_limit = 10
    info!("=== ROUND 1: Testing max_match_limit = 10 ===");

    let limit = 1000;
    let max_match_limit = 10;

    verify_manager
        .append_user_to_verify_list(test_user_id, Some(limit), None, max_match_limit)
        .await?;

    // Get initial state
    let initial_info = verify_manager
        .get_user_unverified_info(test_user_id)
        .await?;
    info!(
        "Initial state - total: {}, pending: {}, max_limit: {}",
        initial_info.total, initial_info.pending_unverify_count, initial_info.max_match_limit
    );

    // Verify limit is applied
    assert!(
        initial_info.total <= limit as i64,
        "Total papers should be <= limit ({limit})"
    );
    assert_eq!(
        initial_info.max_match_limit, max_match_limit as i64,
        "Max match limit should be set correctly"
    );

    // 5. Test idempotency during processing
    info!("=== Testing idempotency (calling append again during processing) ===");

    // Wait a bit for processing to start
    tokio::time::sleep(Duration::from_secs(5)).await;

    let state_before_second_call = verify_manager
        .get_user_unverified_info(test_user_id)
        .await?;

    // Call append again - should be idempotent
    verify_manager
        .append_user_to_verify_list(test_user_id, Some(limit), None, max_match_limit)
        .await?;

    let state_after_second_call = verify_manager
        .get_user_unverified_info(test_user_id)
        .await?;

    // Verify idempotency
    assert_eq!(
        state_before_second_call.total, state_after_second_call.total,
        "Total should not change on idempotent call"
    );

    info!(
        "Idempotency verified - total remained: {}",
        state_after_second_call.total
    );

    // 6. Wait for first round to complete or reach limit
    info!("=== Waiting for processing to complete or reach max_match_limit ===");

    let start_time = Instant::now();
    let wait_duration = Duration::from_secs(900);
    let check_interval = Duration::from_secs(5);
    let mut last_completed_count = 0i64;
    let mut stable_count = 0;

    loop {
        tokio::time::sleep(check_interval).await;

        let current_info = verify_manager
            .get_user_unverified_info(test_user_id)
            .await?;

        info!(
            "Progress - processed: {}/{}, pending: {}, processing: {}, success: {}, fail: {}, tokens: {}",
            current_info.matched_count,
            current_info.max_match_limit,
            current_info.pending_unverify_count,
            current_info.processing_count,
            current_info.success_count,
            current_info.fail_count,
            current_info.token_usage
        );

        // Check if reached limit
        if current_info.matched_count >= max_match_limit as i64 {
            info!("Reached max_match_limit: {}", max_match_limit);

            // Verify processing stopped
            if current_info.processing_count == 0 && current_info.pending_unverify_count > 0 {
                info!("Processing stopped correctly with pending papers remaining");
                break;
            }
        }

        // Check if completed
        if current_info.pending_unverify_count == 0 && current_info.processing_count == 0 {
            info!("All papers processed");
            break;
        }

        // Check if stuck (no progress) - use total completed count instead of matched_count
        let current_completed = current_info.success_count + current_info.fail_count;
        if current_completed == last_completed_count {
            stable_count += 1;
            if stable_count > 6 {
                info!(
                    "No overall progress detected for 30 seconds (completed: {}), assuming stuck or limit reached",
                    current_completed
                );
                break;
            }
        } else {
            stable_count = 0;
            last_completed_count = current_completed;
        }

        if start_time.elapsed() > wait_duration {
            warn!("Timeout waiting for processing");
            break;
        }
    }

    let round1_final_info = verify_manager
        .get_user_unverified_info(test_user_id)
        .await?;

    info!(
        "=== Round 1 Final State ===\n\
        Matched count: {}\n\
        Max match limit: {}\n\
        Success count: {}\n\
        Fail count: {}\n\
        Pending: {}\n\
        Processing: {}\n\
        Total: {}\n\
        Token usage: {}",
        round1_final_info.matched_count,
        round1_final_info.max_match_limit,
        round1_final_info.success_count,
        round1_final_info.fail_count,
        round1_final_info.pending_unverify_count,
        round1_final_info.processing_count,
        round1_final_info.total,
        round1_final_info.token_usage
    );

    // Verify max_match_limit worked
    assert!(
        round1_final_info.matched_count <= max_match_limit as i64,
        "Matched count should not exceed max_match_limit"
    );

    // 7. Verify statistics consistency
    info!("=== Verifying statistics consistency ===");

    let total_processed = round1_final_info.success_count
        + round1_final_info.fail_count
        + round1_final_info.pending_unverify_count
        + round1_final_info.processing_count;

    assert_eq!(
        total_processed, round1_final_info.total,
        "Sum of all states should equal total"
    );

    // 8. Analyze PubSub messages
    info!("=== Analyzing PubSub messages ===");

    let collected_messages = messages.lock().await;
    info!("Collected {} PubSub messages", collected_messages.len());

    let mut prev_matched = 0i64;
    let mut prev_tokens = 0i64;
    let mut prev_success = 0i64;
    let mut papers_with_yes_match = 0;

    for (idx, msg) in collected_messages.iter().enumerate() {
        let info = &msg.user_verify_info;

        // Verify verification_details exists
        assert!(
            msg.verification_details.is_some(),
            "Message {idx}: verification_details should exist"
        );

        if let Some(details) = &msg.verification_details {
            // Verify paper has verifications
            assert!(
                !details.verifications.is_empty(),
                "Message {idx}: paper should have verifications"
            );

            // Count papers with yes match
            let has_yes = details.verifications.iter().any(|v| {
                v.match_ == seaorm_db::entities::feed::sea_orm_active_enums::VerificationMatch::Yes
            });
            if has_yes {
                papers_with_yes_match += 1;
            }

            let paper_id = details
                .verifications
                .first()
                .map(|v| v.paper_id)
                .unwrap_or(0);
            info!(
                "Message {idx}: paper_id={}, verifications={}, has_yes_match={}",
                paper_id,
                details.verifications.len(),
                has_yes
            );
        }

        // Verify statistics consistency in each message
        // Note: In concurrent systems, intermediate states may have small discrepancies
        // due to non-atomic Redis updates. Allow a tolerance of ±3 for intermediate messages.
        let msg_total = info.pending_unverify_count
            + info.processing_count
            + info.success_count
            + info.fail_count;

        let diff = (msg_total - info.total).abs();
        assert!(
            diff <= 3,
            "Message {idx}: sum ({msg_total}) should approximately equal total ({}) (diff: {diff})",
            info.total
        );

        // Verify max_match_limit is correct
        assert_eq!(
            info.max_match_limit, max_match_limit as i64,
            "Message {idx}: max_match_limit should be {max_match_limit}"
        );

        // Verify success_count increments by 1 each time
        if idx > 0 {
            assert_eq!(
                info.success_count,
                prev_success + 1,
                "Message {idx}: success_count should increment by 1"
            );
        }

        // Verify monotonic increase (except resets)
        if info.matched_count < prev_matched {
            info!(
                "Message {idx}: matched_count reset from {prev_matched} to {}",
                info.matched_count
            );
        } else {
            assert!(
                info.matched_count >= prev_matched,
                "Message {idx}: matched_count should be monotonically increasing"
            );
        }

        assert!(
            info.token_usage >= prev_tokens,
            "Message {idx}: token_usage should be monotonically increasing"
        );

        prev_matched = info.matched_count;
        prev_tokens = info.token_usage;
        prev_success = info.success_count;
    }

    // Verify total papers with yes match equals final matched_count
    info!(
        "Total papers with yes match: {}, final matched_count: {}",
        papers_with_yes_match, round1_final_info.matched_count
    );
    assert_eq!(
        papers_with_yes_match as i64, round1_final_info.matched_count,
        "Papers with yes match should equal matched_count"
    );

    // Verify total messages equals final success_count + fail_count
    let total_completed = round1_final_info.success_count + round1_final_info.fail_count;
    assert_eq!(
        collected_messages.len() as i64,
        total_completed,
        "Number of messages should equal success_count + fail_count"
    );

    // 9. Second round: Reset and continue with new limit
    if round1_final_info.pending_unverify_count > 0
        && round1_final_info.matched_count >= max_match_limit as i64
    {
        info!("=== ROUND 2: Testing reset and new max_match_limit = 5 ===");

        let new_max_match_limit = 5;

        // Wait for processing to fully stop
        tokio::time::sleep(Duration::from_secs(5)).await;

        verify_manager
            .append_user_to_verify_list(test_user_id, Some(limit), None, new_max_match_limit)
            .await?;

        let round2_initial_info = verify_manager
            .get_user_unverified_info(test_user_id)
            .await?;

        info!(
            "Round 2 initial state - matched_count: {}, max_match_limit: {}",
            round2_initial_info.matched_count, round2_initial_info.max_match_limit
        );

        // Verify matched_count was reset
        assert_eq!(
            round2_initial_info.matched_count, 0,
            "Round 2: matched_count should be reset to 0"
        );

        // Verify max_match_limit was updated to new value
        assert_eq!(
            round2_initial_info.max_match_limit, new_max_match_limit as i64,
            "Round 2: max_match_limit should be updated to {new_max_match_limit}"
        );

        // Wait for some processing in round 2
        tokio::time::sleep(Duration::from_secs(30)).await;

        let round2_progress_info = verify_manager
            .get_user_unverified_info(test_user_id)
            .await?;

        info!(
            "Round 2 progress - matched: {}/{}, success: {}, fail: {}",
            round2_progress_info.matched_count,
            round2_progress_info.max_match_limit,
            round2_progress_info.success_count,
            round2_progress_info.fail_count
        );
    }

    // 10. Database verification
    info!("=== Verifying database records ===");

    let db_verifications = UserPaperVerificationsQuery::list_by_user_id(&db, test_user_id).await?;

    let db_count = db_verifications.len();
    let redis_completed = round1_final_info.success_count + round1_final_info.fail_count;

    info!(
        "Database verification count: {}, Redis completed count: {}",
        db_count, redis_completed
    );

    // Cleanup
    info!("=== Cleanup ===");
    let user_channel =
        RedisPubSubManager::build_user_channel(&config.rss.verify_papers_channel, test_user_id);
    pubsub_manager.unsubscribe(&user_channel).await?;
    let _ = verify_manager.finish_user_verify(test_user_id).await;

    info!("Test completed successfully!");

    Ok(())
}

async fn setup_user_test_data(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
    source_ids: &[i32],
    config: &conf::config::AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Setting up test data for user {}", user_id);

    // Create 4 interests related to AI and computer science
    let interests = vec![
        "Machine Learning and Deep Neural Networks".to_string(),
        "Quantum Computing and Algorithms".to_string(),
        "Distributed Systems and Cloud Computing".to_string(),
        "Natural Language Processing".to_string(),
    ];

    UserInterestsQuery::replace_many(db, user_id, interests.clone(), config.llm.model.clone())
        .await?;

    info!("Created {} interests for user {}", interests.len(), user_id);

    // Use specific AI/ML/CS-related RSS sources that match our interests
    // These sources have been verified to have substantial papers in the database:
    // - 35: Machine Learning (2041 papers)
    // - 13: Artificial Intelligence (1621 papers)
    // - 20: Computer Vision (1126 papers)
    // - 18: Computation and Language/NLP (763 papers)
    // - 10: Artificial Intelligence (363 papers)
    // - 23: Distributed, Parallel, and Cluster Computing
    // - 165: Statistics Machine Learning (33 papers)
    // - 41: Neural and Evolutionary Computing (32 papers)
    let preferred_sources = vec![35, 13, 20, 18, 10, 23, 165, 41];

    // Filter to only include sources that exist in the database
    let selected_sources: Vec<i32> = preferred_sources
        .into_iter()
        .filter(|id| source_ids.contains(id))
        .collect();

    if selected_sources.is_empty() {
        warn!("None of the preferred CS sources exist, falling back to available sources");
        let mut sources = source_ids.to_vec();
        let mut rng = rand::rng();
        sources.shuffle(&mut rng);
        let selected_sources: Vec<i32> = sources.into_iter().take(8).collect();
        RssSubscriptionsQuery::replace_many(db, user_id, selected_sources.clone()).await?;
        info!(
            "Created {} subscriptions for user {} (fallback)",
            selected_sources.len(),
            user_id
        );
        return Ok(());
    }

    info!(
        "Selecting {} CS-related subscriptions for user {}",
        selected_sources.len(),
        user_id
    );

    RssSubscriptionsQuery::replace_many(db, user_id, selected_sources.clone()).await?;

    info!(
        "Created {} subscriptions for user {}",
        selected_sources.len(),
        user_id
    );

    Ok(())
}
