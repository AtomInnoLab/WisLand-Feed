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

/// Test concurrent calls to replace_many method for the same user
/// This should test data consistency to ensure the final result is correct
#[tokio::test]
async fn test_same_user_concurrent_replace_many() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let db = get_db().await.clone();
    let config = app_config();
    let test_user_id = 999999; // Use a test user ID

    info!("Starting test for concurrent replace_many calls for the same user");

    // Clean up test data
    cleanup_test_data(&db, test_user_id).await?;

    let barrier = Arc::new(Barrier::new(3)); // 3 concurrent tasks
    let mut handles = vec![];

    // Create 3 concurrent tasks, each setting different interest lists
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

            info!(task_id = i, ?interests, "Task {} preparing to set interests", i);

            // Wait for all tasks to start simultaneously
            barrier_clone.wait().await;

            info!(task_id = i, "Task {} starting to execute replace_many", i);
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
                "Task {} completed",
                i
            );

            match result {
                Ok(ids) => {
                    info!(task_id = i, ?ids, "Task {} succeeded, returned ID list", i);

                    // Output current interest list
                    let current_interests =
                        UserInterestsQuery::list_by_user_id(&db_clone, test_user_id).await?;
                    info!(
                        task_id = i,
                        ?current_interests,
                        "Current interest list after task {} execution",
                        i
                    );

                    Ok(ids)
                }
                Err(e) => {
                    warn!(task_id = i, error = %e, "Task {} failed", i);
                    Err(e)
                }
            }
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    let results: Result<Vec<_>, _> = futures::future::try_join_all(handles).await;
    let _task_results = results?;

    info!("All concurrent tasks completed, starting result verification");

    // Verify final result
    let final_interests = UserInterestsQuery::list_by_user_id(&db, test_user_id).await?;
    info!(
        count = final_interests.len(),
        ?final_interests,
        "Final interest list"
    );

    // Verify data consistency
    assert_eq!(final_interests.len(), 3, "Should have 3 interests in the end");

    // Verify uniqueness of interest names
    let interest_names: Vec<String> = final_interests.into_iter().map(|m| m.interest).collect();
    let unique_names: std::collections::HashSet<String> = interest_names.iter().cloned().collect();
    assert_eq!(unique_names.len(), 3, "All interest names should be unique");

    // Clean up test data
    cleanup_test_data(&db, test_user_id).await?;

    info!("Same user concurrent test completed");
    Ok(())
}

/// Clean up test data
async fn cleanup_test_data(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Output interest list before cleanup
    let before_interests = UserInterestsQuery::list_by_user_id(db, user_id).await?;
    info!(
        user_id,
        ?before_interests,
        "Interest list before cleanup for user {}",
        user_id
    );

    // Use replace_many to clear all user interests
    let config = app_config();
    UserInterestsQuery::replace_many(db, user_id, vec![], config.llm.model.clone()).await?;

    // Output interest list after cleanup
    let after_interests = UserInterestsQuery::list_by_user_id(db, user_id).await?;
    info!(
        user_id,
        ?after_interests,
        "Interest list after cleanup for user {}",
        user_id
    );

    info!(user_id, "Cleanup of test data for user {} completed", user_id);
    Ok(())
}
