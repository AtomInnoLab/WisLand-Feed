use super::FEED_TAG;
use crate::{
    middlewares::auth::User,
    model::{base::ApiResponse, page::Page},
    state::app_state::AppState,
};
use axum::extract::{Query, State};
use common::{error::api_error::*, prelude::ApiCode};
use seaorm_db::query::feed::{
    rss_papers::{GetUnverifiedPaperIdsParams, RssPaperData, RssPapersQuery},
    utils::UserUnverifiedPapers,
};
use serde::Deserialize;
use snafu::ResultExt;
use utoipa::ToSchema;

// #[derive(Debug, Deserialize, ToSchema)]
// pub struct AllVerifiedPapersRequest {
//     #[serde(flatten)]
//     pub pagination: Page,
//     pub channel: Option<String>,
//     pub matches: Option<Vec<VerificationMatch>>,
//     pub user_interest_ids: Option<Vec<i64>>,
//     pub time_range: Option<TimeRangeParam>,
//     pub ignore_time_range: Option<bool>,
//     pub keyword: Option<String>,
// }

#[derive(Debug, Deserialize, ToSchema)]
pub struct PapersRequest {
    #[serde(flatten)]
    pub pagination: Page,
    pub channel: Option<String>,
    pub keyword: Option<String>,
}

#[utoipa::path(
    get,
    path = "/unverified-papers",
    request_body = PapersRequest,
    responses(
        (status = 200, body = Vec<RssPaperData>),
    ),
    tag = FEED_TAG,
)]
pub async fn unverified_papers(
    State(state): State<AppState>,
    User(user): User,
    Query(payload): Query<PapersRequest>,
) -> Result<ApiResponse<Vec<RssPaperData>>, ApiError> {
    tracing::info!("get papers");

    let rss_papers = RssPapersQuery::get_unverified_papers(
        &state.conn,
        user.id,
        GetUnverifiedPaperIdsParams {
            offset: Some(payload.pagination.offset()),
            limit: Some(payload.pagination.page_size()),
            channel: payload.channel,
            keyword: payload.keyword,
        },
    )
    .await
    .context(DbErrSnafu {
        stage: "get-papers",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;

    Ok(ApiResponse::data(rss_papers))
}
