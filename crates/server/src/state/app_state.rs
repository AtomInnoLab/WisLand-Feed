use std::sync::Arc;

use axum::extract::FromRef;
use conf::config::{AppConfig, app_config};
use sea_orm::DatabaseConnection;
use seaorm_db::connection::get_db;
use tokio::signal::{self, unix::SignalKind};
use tracing::*;

#[derive(Clone)]
pub struct AppState {
    pub conn: DatabaseConnection,
    pub redis: RedisService,
    pub config: Arc<AppConfig>,
}

#[derive(Clone)]
pub struct RedisService {
    pub pool: bb8::Pool<bb8_redis::RedisConnectionManager>,
    pub apalis_conn: apalis_redis::ConnectionManager,
}
impl AppState {
    pub async fn new() -> Self {
        let config = app_config();
        AppState {
            conn: get_db().await.clone(),
            redis: RedisService {
                pool: connect_redis(&config.agent_redis).await,
                apalis_conn: apalis_redis::connect(config.agent_redis.url.as_str())
                    .await
                    .expect("Could not connect redis"),
            },
            config,
        }
    }
}

impl FromRef<AppState> for DatabaseConnection {
    fn from_ref(input: &AppState) -> Self {
        input.conn.clone()
    }
}

pub async fn graceful_shutdown(_state: AppState) {
    // Wait for Ctrl+C signal
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
        }
        _ = async {
            let mut sigterm = signal::unix::signal(SignalKind::terminate())
                .expect("Failed to listen to SIGTERM");
            sigterm.recv().await;
            info!("Received SIGTERM, shutting down...");
        } => {}
    }

    info!("Bye");
}

pub async fn connect_redis(
    config: &conf::config::AgentRedisConfig,
) -> bb8::Pool<bb8_redis::RedisConnectionManager> {
    let manager =
        bb8_redis::RedisConnectionManager::new(config.url.clone()).expect("Invalid Redis URL");
    bb8::Pool::builder()
        .max_size(config.pool_size)
        .build(manager)
        .await
        .expect("Failed to create Redis connection pool")
}
