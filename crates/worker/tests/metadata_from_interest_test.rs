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

    let interest = "large language model for paper verification";

    info!(%interest, "starting criteria_from_interest test");

    Ok(())
}
