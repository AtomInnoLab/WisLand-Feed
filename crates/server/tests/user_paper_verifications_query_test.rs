use seaorm_db::connection::get_db;
use seaorm_db::query::feed::user_paper_verifications::UserPaperVerificationsQuery;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;


#[tokio::test]
async fn test_exists_by_user_paper_interest() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .compact()
        .try_init();
    dotenvy::dotenv().ok();

    info!("Starting test_exists_by_user_paper_interest");

    // 获取数据库连接
    let db = get_db().await.clone();
    info!("Database connection acquired");

    // 查询现有测试数据 - 从 user_paper_verifications 表中获取一条未被软删除的记录
    let existing_records = UserPaperVerificationsQuery::list_by_user_id(&db, 1).await?;

    if existing_records.is_empty() {
        warn!("No user_paper_verifications records found in database, skipping test");
        return Ok(());
    }

    // 取第一条记录作为测试数据
    let test_record = &existing_records[0];
    let test_user_id = test_record.user_id;
    let test_paper_id = test_record.paper_id;
    let test_user_interest_id = test_record.user_interest_id;

    info!(
        user_id = test_user_id,
        paper_id = test_paper_id,
        user_interest_id = test_user_interest_id,
        "Using existing record for testing"
    );

    // 场景1：测试已存在的记录
    info!("Testing scenario 1: existing record");
    let exists_result = UserPaperVerificationsQuery::exists_by_user_paper_interest(
        &db,
        test_user_id,
        test_paper_id,
        test_user_interest_id,
    )
    .await?;

    assert!(exists_result, "Should return true for existing record");
    info!("✓ Scenario 1 passed: existing record correctly identified");

    // 场景2：测试不存在的记录
    info!("Testing scenario 2: non-existing record");
    let non_existing_user_interest_id = 999999999;
    let not_exists_result = UserPaperVerificationsQuery::exists_by_user_paper_interest(
        &db,
        test_user_id,
        test_paper_id,
        non_existing_user_interest_id,
    )
    .await?;

    assert!(
        !not_exists_result,
        "Should return false for non-existing record"
    );
    info!("✓ Scenario 2 passed: non-existing record correctly identified");

    info!("All test scenarios passed successfully!");
    Ok(())
}
