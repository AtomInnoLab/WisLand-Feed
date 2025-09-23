use conf::config::app_config;
use dotenvy::dotenv;
use feed::manager;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    // 加载配置并输出关键启动信息
    let cfg = app_config();
    info!(target: "feed", redis_prefix = %cfg.rss.redis_prefix, "Starting feed workers");

    // 初始化并启动 workers（Monitor 注册在 init 内部完成）
    manager::entry::init().await?;
    info!(target: "feed", "Workers started and running");
    // 阻塞运行：Apalis Monitor 内部托管，当前进程保持存活
    // 如果需要显式阻塞，可在此添加一个 pending future
    futures::future::pending::<()>().await;
    Ok(())
}
