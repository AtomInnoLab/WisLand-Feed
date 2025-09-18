use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::app_state::AppState;

pub mod feeds;
pub mod interests;
pub mod rss;
pub mod subscriptions;

pub(crate) const FEED_TAG: &str = "feed";

pub fn feed_routers() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(rss::rss))
        .routes(routes!(rss::rss_detail))
        .routes(routes!(rss::rss_create))
        .routes(routes!(rss::rss_delete))
        .routes(routes!(subscriptions::subscriptions))
        .routes(routes!(subscriptions::batch_subscriptions))
        .routes(routes!(subscriptions::subscriptions_create_one))
        .routes(routes!(subscriptions::subscriptions_delete_one))
        .routes(routes!(interests::interests))
        .routes(routes!(interests::set_interests))
        .routes(routes!(feeds::verify))
        .routes(routes!(feeds::verify_detail))
        .routes(routes!(feeds::all_verified_papers))
        .routes(routes!(feeds::papers_make_read))
        .routes(routes!(feeds::unverified_count_info))
        .routes(routes!(feeds::unread_count))
        .routes(routes!(feeds::batch_delete))
}
