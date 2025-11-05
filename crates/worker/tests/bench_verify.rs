use std::time::{Duration, Instant};

use conf::config::app_config;
use feed::parsers::arxiv::convert_rss_paper_model_to_paper;
use feed::workers::{base::RedisService, verify_user_papers::run_verify_with_input};
use protocol::tasks::verify::Criteria;
use sea_orm::{EntityTrait, QueryOrder, QuerySelect};
use seaorm_db::connection::get_db;
use seaorm_db::entities::feed::rss_papers;
use seaorm_db::query::feed::user_interests::UserInterestsQuery;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::test]
async fn bench_verify_by_order() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .compact()
        .try_init();
    dotenvy::dotenv().ok();

    // load config
    let config = app_config();
    let model_name = config.llm.model.clone();
    let redis_url = config.rss.feed_redis.url.clone();

    // db connection
    let db = get_db().await.clone();

    // redis pool + apalis
    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            warn!(error = %err, "skip bench: invalid REDIS URL");
            return Ok(());
        }
    };
    let pool = match bb8::Pool::builder()
        .max_size(config.rss.feed_redis.pool_size)
        .connection_timeout(Duration::from_secs(3))
        .build(manager)
        .await
    {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "skip bench: cannot connect redis");
            return Ok(());
        }
    };
    let apalis_conn = apalis_redis::connect(redis_url.as_str()).await?;

    let redis_service = RedisService { pool, apalis_conn };

    // 1) load interests for user_id=1, take first 10
    let interests_items = UserInterestsQuery::list_by_user_id(&db, 1)
        .await
        .map_err(|e| anyhow::anyhow!("load interests: {e}"))?;
    let mut interest_texts: Vec<String> = interests_items.into_iter().map(|m| m.interest).collect();
    if interest_texts.len() < 10 {
        warn!(
            count = interest_texts.len(),
            "not enough interests, need >=10"
        );
    }
    interest_texts.truncate(10);

    // group into 5 groups (2 each) -> Vec<Vec<Criteria>>
    let mut interest_groups: Vec<Vec<Criteria>> = Vec::new();
    for i in (0..interest_texts.len()).step_by(2) {
        if i + 1 < interest_texts.len() {
            interest_groups.push(vec![
                Criteria::String(interest_texts[i].clone()),
                Criteria::String(interest_texts[i + 1].clone()),
            ]);
        }
    }
    // ensure exactly 5 groups when there are enough interests
    if interest_groups.len() > 5 {
        interest_groups.truncate(5);
    }

    // 2) load 100 papers ordered by id asc via Entity (stable and available)
    let papers_models: Vec<rss_papers::Model> = rss_papers::Entity::find()
        .order_by_asc(rss_papers::Column::Id)
        .limit(100)
        .all(&db)
        .await
        .map_err(|e| anyhow::anyhow!("load papers: {e}"))?;

    if papers_models.is_empty() || interest_groups.is_empty() {
        warn!(
            papers = papers_models.len(),
            groups = interest_groups.len(),
            "insufficient data for bench"
        );
        return Ok(());
    }

    // convert to Paper inputs
    let papers: Vec<search::web::scholar::paper::Paper> = papers_models
        .iter()
        .map(convert_rss_paper_model_to_paper)
        .collect();

    // warmup (1 paper x 1 group)
    let warmup_paper = papers[0].clone();
    let warmup_group = interest_groups[0].clone();
    let _ = run_verify_with_input(
        db.clone(),
        RedisService {
            pool: redis_service.pool.clone(),
            apalis_conn: redis_service.apalis_conn.clone(),
        },
        search::agent::verify::ToBeVerified::Paper(Box::new(warmup_paper)),
        warmup_group,
        model_name.clone(),
        "rss feed verify benchmark",
    )
    .await;

    // compute repeats to reach >= 5000 calls per strategy
    let base_calls = papers.len() * interest_groups.len();
    let repeats = (5000usize).div_ceil(base_calls);

    // benchmark 1: by paper (outer: papers, inner: groups), repeated
    let start = Instant::now();
    for _ in 0..repeats {
        for paper in &papers {
            for group in &interest_groups {
                let _ = run_verify_with_input(
                    db.clone(),
                    RedisService {
                        pool: redis_service.pool.clone(),
                        apalis_conn: redis_service.apalis_conn.clone(),
                    },
                    search::agent::verify::ToBeVerified::Paper(Box::new(paper.clone())),
                    group.clone(),
                    model_name.clone(),
                    "rss feed verify benchmark",
                )
                .await;
            }
        }
    }
    let dur_by_paper = start.elapsed();

    // benchmark 2: by interest (outer: groups, inner: papers), repeated
    let start = Instant::now();
    for _ in 0..repeats {
        for group in &interest_groups {
            for paper in &papers {
                let _ = run_verify_with_input(
                    db.clone(),
                    RedisService {
                        pool: redis_service.pool.clone(),
                        apalis_conn: redis_service.apalis_conn.clone(),
                    },
                    search::agent::verify::ToBeVerified::Paper(Box::new(paper.clone())),
                    group.clone(),
                    model_name.clone(),
                    "rss feed verify benchmark",
                )
                .await;
            }
        }
    }
    let dur_by_interest = start.elapsed();

    let calls = base_calls * repeats;
    info!(
        calls,
        ms_by_paper = %dur_by_paper.as_millis(),
        ms_by_interest = %dur_by_interest.as_millis(),
        qps_by_paper = (calls as f64) / (dur_by_paper.as_secs_f64().max(1e-6)),
        qps_by_interest = (calls as f64) / (dur_by_interest.as_secs_f64().max(1e-6)),
        "bench finished"
    );

    Ok(())
}
