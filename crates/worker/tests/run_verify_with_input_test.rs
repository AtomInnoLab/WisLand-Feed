use std::time::Duration;

use feed::workers::{base::RedisService, verify_user_papers::run_verify_with_input};
use protocol::tasks::verify::Criteria;
use seaorm_db::connection::get_db;
use search::web::scholar::paper::{Paper, PaperSource};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use dotenvy::dotenv;

#[tokio::test]
async fn test_run_verify_with_input_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .compact()
        .try_init();
    dotenv().ok();

    info!("starting test_run_verify_with_input_smoke");
    // prepare db connection (ensure DB init)
    let db = get_db().await.clone();
    debug!("db connection acquired");

    let redis_url = std::env::var("REDIS_URL")
        .or_else(|_| std::env::var("APP_AGENT_REDIS__URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:16379/0".to_string());
    info!(%redis_url, "using redis url");
    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            warn!(error = %err, "skip test: invalid REDIS_URL");
            return Ok(());
        }
    };

    let pool = match bb8::Pool::builder()
        .max_size(1)
        .connection_timeout(Duration::from_secs(2))
        .build(manager)
        .await
    {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "skip test: cannot connect redis");
            return Ok(());
        }
    };
    info!("redis pool created");

    // build a minimal Paper input
    let paper = Paper::builder()
        .id("1")
        .title("Test Paper about Large Language Models")
        .authors(vec!["Alice".to_string(), "Bob".to_string()])
        .year(None)
        .citations(0)
        .paper_id(Vec::new())
        .abstract_text(Some("We study LLM verification.".to_string()))
        .url(Some("https://example.com/paper".to_string()))
        .cites_ids(Vec::new())
        .source(PaperSource::GoogleScholar)
        .uuid(Uuid::new_v4())
        .venue(Some("Nowhere 2025".to_string()))
        .pdf_url(Some("https://example.com/paper.pdf".to_string()))
        .cites(None)
        .doi(None)
        .affiliations(Some("Affiliation 1".to_string()))
        .conference_journal(None)
        .conference_journal_type(None)
        .research_field(None)
        .build();
    info!(title = %paper.title, authors = %paper.authors.join(", "), venue = ?paper.venue, url = ?paper.url, "constructed paper input");

    // simple criteria
    let criteria = vec![Criteria::String("the paper is about LLM".to_string())];
    let original_query = "paper verification for llm";
    debug!(?criteria, %original_query, "criteria and query prepared");

    // execute
    info!("executing run_verify_with_input");
    let result = run_verify_with_input(
        db,
        RedisService {
            pool,
            apalis_conn: apalis_redis::connect(redis_url.as_str())
                .await
                .expect("Could not connect redis"),
        },
        search::agent::verify::ToBeVerified::Paper(Box::new(paper)),
        criteria,
        "WisModel-20250821-8B-strict".to_string(),
        original_query,
    )
    .await?;
    info!(?result, "verify result received");

    Ok(())
}
