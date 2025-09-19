use std::time::Duration;

use feed::workers::base::RedisService;
use seaorm_db::connection::get_db;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::test]
async fn test_metadata_from_interest_returns_json_or_empty()
-> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .compact()
        .try_init();

    let _db = get_db().await.clone();

    let redis_url = std::env::var("REDIS_URL")
        .or_else(|_| std::env::var("APP_AGENT_REDIS__URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:16379/0".to_string());
    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            eprintln!("skip test: invalid REDIS_URL ({err})");
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
            eprintln!("skip test: cannot connect redis ({err})");
            return Ok(());
        }
    };

    let interest = "large language model for paper verification";

    info!(%interest, "starting criteria_from_interest test");

    let result = feed::workers::update_user_interest_metadata::verify_from_interest(
        RedisService {
            pool,
            apalis_conn: apalis_redis::connect(redis_url.as_str())
                .await
                .expect("Could not connect redis"),
        },
        interest,
    )
    .await;
    assert!(result.is_ok(), "criteria_from_interest should not error");
    let verify = result.unwrap();
    info!(?verify, "verify returned");
    Ok(())
}
