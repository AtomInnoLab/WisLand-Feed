use std::{thread, time::Duration};

use apalis::prelude::Storage;
use apalis_redis::RedisStorage;
use conf::config::app_config;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use seaorm_db::{connection::get_db, entities::web::feed::rss_job_logs};
use tracing::{debug, error, info, instrument};
use tracing_subscriber::EnvFilter;

// Import worker payloads
use feed::workers::pull_rss_source::PullRssSourceInput;
use feed::workers::verify_user_papers::VerifyAllUserPapersInput;

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
    });
}

#[instrument(skip(payload))]
async fn push_verify_all_papers_job(payload: VerifyAllUserPapersInput) -> anyhow::Result<()> {
    let cfg = app_config();
    info!(redis_url = %cfg.agent_redis.url, "connecting to agent redis");
    let conn = apalis_redis::connect(cfg.agent_redis.url.as_str()).await?;
    info!("redis connection established");
    let mut storage: RedisStorage<VerifyAllUserPapersInput> = RedisStorage::new(conn);
    info!(
        user_id = payload.user_id,
        max_prompt_number = payload.max_prompt_number,
        max_rss_paper = payload.max_rss_paper,
        has_channel = payload.channel,
        "pushing verify_all_user_papers job"
    );
    storage.push(payload).await?;
    info!("verify_all_user_papers job pushed");
    Ok(())
}

#[instrument(skip(payload))]
async fn push_pull_rss_source_job(payload: PullRssSourceInput) -> anyhow::Result<()> {
    let cfg = app_config();
    info!(redis_url = %cfg.agent_redis.url, "connecting to agent redis");
    let conn = apalis_redis::connect(cfg.agent_redis.url.as_str()).await?;
    info!("redis connection established");
    let mut storage: RedisStorage<PullRssSourceInput> = RedisStorage::new(conn);
    info!("pushing pull_rss_source job");
    storage.push(payload).await?;
    info!("pull_rss_source job pushed");
    Ok(())
}

#[instrument(skip_all, fields(task_type = %task_type))]
async fn count_logs(task_type: &str) -> anyhow::Result<i64> {
    let db = get_db().await.clone();
    let count = rss_job_logs::Entity::find()
        .filter(rss_job_logs::Column::TaskType.eq(task_type))
        .count(&db)
        .await?;
    let count_i64 = count as i64;
    debug!(count = count_i64, "counted logs");
    Ok(count_i64)
}

/// Poll until at least `min_increase` logs for the given task_type appear.
#[instrument(skip_all, fields(task_type = %task_type, baseline, min_increase, max_wait_ms = max_wait.as_millis()))]
async fn wait_for_new_logs(
    task_type: &str,
    baseline: i64,
    min_increase: i64,
    max_wait: Duration,
) -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    loop {
        let now = count_logs(task_type).await?;
        let delta = now - baseline;
        if delta >= min_increase {
            info!(final_count = now, delta, "log increase threshold met");
            return Ok(());
        }
        if start.elapsed() > max_wait {
            error!(
                elapsed_ms = start.elapsed().as_millis() as u64,
                now, delta, "timeout waiting for new logs"
            );
            anyhow::bail!("timeout waiting for logs for {}", task_type);
        }
        debug!(now, delta, "not enough logs yet; sleeping 500ms");
        thread::sleep(Duration::from_millis(500));
    }
}

#[tokio::test]
async fn test_verify_all_papers_logs() -> anyhow::Result<()> {
    init_test_tracing();
    // Given a running worker service, pushing a job should create at least a start log
    let task_type = "verify_user_papers";
    let before = count_logs(task_type).await?;
    info!(%task_type, before, "starting test_verify_all_papers_logs");

    // Push a small job. Even if the job fails internally, the wrapper logs should appear.
    let payload = VerifyAllUserPapersInput {
        user_id: 0, // a harmless id; success not required
        channel: "".to_string(),
        max_prompt_number: 1,
        max_rss_paper: 1,
    };
    push_verify_all_papers_job(payload).await?;
    info!("job pushed; waiting for logs to increase");

    // Wait for at least one new log (start); usually two (start + success/failed)
    wait_for_new_logs(task_type, before, 1, Duration::from_secs(30)).await?;
    let after = count_logs(task_type).await?;
    info!(%task_type, before, after, delta = after - before, "logs increased after verify job");
    Ok(())
}

#[tokio::test]
async fn test_pull_rss_source() -> anyhow::Result<()> {
    init_test_tracing();
    let task_type = "pull_rss_source";
    let before = count_logs(task_type).await?;
    info!(%task_type, before, "starting test_pull_rss_source");

    // Push a small job. Even if the job fails internally, the wrapper logs should appear.
    let payload = PullRssSourceInput {};
    push_pull_rss_source_job(payload).await?;
    info!("job pushed; waiting for logs to increase");
    wait_for_new_logs(task_type, before, 1, Duration::from_secs(30)).await?;
    let after = count_logs(task_type).await?;
    info!(%task_type, before, after, delta = after - before, "logs increased after pull_rss_source job");
    Ok(())
}
