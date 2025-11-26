use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::app_state::AppState;

pub mod detail;

pub fn internal_routers() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(detail::get_detail))
}
