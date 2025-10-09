use crate::{
    middlewares::*,
    routers::{
        feed::{self},
        health::{self, handler_404},
    },
    state::app_state::AppState,
};
use ::feed::dispatch;
use ::feed::workers::verify_user_scheduler::VerifyUserSchedulerInput;
use axum::{Router, middleware};
use common::{error::api_error::*, prelude::ApiCode};
use conf::config::app_config;
use tower_http::catch_panic::CatchPanicLayer;
use tracing::info;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "wisland-feed", description = "Agent Service Name"),
    )
)]
struct ApiDoc;

pub async fn build_app() -> Result<(Router, AppState), ApiError> {
    // get app config
    let config = app_config();

    info!("config: {:?}", config);
    // build app state
    let state = AppState::new().await;

    start_verify_user_scheduler_worker(state.redis.apalis_conn.clone()).await?;

    // build the router with OpenAPI documentation
    let url_prefix = config.server.api_prefix.trim_end_matches('/');
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest(url_prefix, health::health_routers())
        .nest(url_prefix, feed::feed_routers())
        .split_for_parts();

    // build the final router with Swagger UI and Scalar documentation
    let router = router
        .merge(
            SwaggerUi::new(format!("{url_prefix}/swagger-ui"))
                .url(format!("{url_prefix}/openapi.json"), api.clone()),
        )
        .merge(Scalar::with_url(format!("{url_prefix}/docs"), api))
        .layer(CatchPanicLayer::custom(PanicHandler)) // panic handler
        // .layer(middleware::from_fn(log::log_response))
        .layer(middleware::from_fn(log::log_request))
        .with_state(state.clone())
        .fallback(handler_404);

    Ok((router, state))
}

pub async fn start_verify_user_scheduler_worker(
    apalis_conn: apalis_redis::ConnectionManager,
) -> Result<(), ApiError> {
    info!("start_verify_user_scheduler_worker");
    dispatch(VerifyUserSchedulerInput {}, apalis_conn)
        .await
        .map_err(|e| ApiError::CustomError {
            message: format!("start_verify_user_scheduler_worker: {e}"),
            code: ApiCode::COMMON_FEED_ERROR,
        })?;
    info!("start_verify_user_scheduler_worker success");
    Ok(())
}
