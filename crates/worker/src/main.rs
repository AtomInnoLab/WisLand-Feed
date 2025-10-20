use conf::config::app_config;
use dotenvy::dotenv;
use feed::manager;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    // Load configuration and output key startup information
    let cfg = app_config();
    info!(target: "feed", redis_prefix = %cfg.rss.feed_redis.redis_prefix, "Starting feed workers"); // Initialize logging
    let _guard = cfg.init_log(true);

    // Initialize and start workers (Monitor registration is completed inside init)
    manager::entry::init().await?;
    info!(target: "feed", "Workers started and running");
    // Blocking run: Apalis Monitor internally managed, current process stays alive
    // If explicit blocking is needed, a pending future can be added here
    futures::future::pending::<()>().await;
    Ok(())
}
