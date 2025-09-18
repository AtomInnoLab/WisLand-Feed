use axum::response::IntoResponse;
use common::{error::api_error::ApiError, prelude::ApiCode};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::app_state::AppState;

#[utoipa::path(
    method(get),
    path = "/health",
    responses(
        (status = OK, description = "Success", body = str, content_type = "text/plain")
    ),
    tag = "Common"
)]
pub async fn health() -> &'static str {
    "ok"
}

pub fn health_routers() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(health))
}

/// 404 handler
pub async fn handler_404() -> impl IntoResponse {
    ApiError::NotFound {
        code: ApiCode {
            http_code: 404,
            code: 200000,
        },
    }
}
