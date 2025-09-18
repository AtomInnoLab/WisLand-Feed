use axum::{Router, middleware};
use common::error::api_error::*;
use conf::config::app_config;
use tower_http::catch_panic::CatchPanicLayer;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    middlewares::*,
    routers::{
        feed::{self},
        health::{self, handler_404},
    },
    state::app_state::AppState,
};

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
    // build app state
    let state = AppState::new().await;

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
