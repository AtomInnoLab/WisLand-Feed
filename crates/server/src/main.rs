use axum::BoxError;
use conf::config::app_config;
use dotenvy::dotenv;
use server::{app::build_app, state::app_state::graceful_shutdown};
use std::net::{IpAddr, SocketAddr};
use tracing::*;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    dotenv().ok();
    let config = app_config();

    let (router, state) = build_app().await?;
    info!("init server successfully");

    let addr = SocketAddr::new(
        config
            .server
            .host
            .parse::<IpAddr>()
            .expect("Invalid host address"),
        config.server.port,
    );

    warn!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router.into_make_service())
        .with_graceful_shutdown(graceful_shutdown(state))
        .await?;

    Ok(())
}
