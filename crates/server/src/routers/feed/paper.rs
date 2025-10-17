use super::FEED_TAG;
use crate::{
    middlewares::auth::User,
    model::{base::ApiResponse, page::Pagination},
    state::app_state::AppState,
};
use axum::extract::{Query, State};
use common::{error::api_error::*, prelude::ApiCode};
use seaorm_db::entities::feed::sea_orm_active_enums::VerificationMatch as VerificationMatchEnum;
use seaorm_db::query::feed::rss_papers::{
    GetUnverifiedPaperIdsParams, RssPaperDataWithDetail, RssPapersQuery,
};
use serde::{Deserialize, Serialize};
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
    /// Page number for pagination (optional)
    pub page: Option<i32>,
    /// Number of items per page (optional)
    pub page_size: Option<i32>,
    pub channel: Option<String>,
    pub keyword: Option<String>,
    pub rss_source_id: Option<i32>,
    #[serde(default = "default_verification_match")]
    pub not_match: Option<VerificationMatchEnum>,
}

fn default_verification_match() -> Option<VerificationMatchEnum> {
    Some(VerificationMatchEnum::Yes)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnverifiedPapersResponse {
    pub pagination: Pagination,
    pub papers: Vec<RssPaperDataWithDetail>,
}

#[utoipa::path(
    get,
    path = "/unverified-papers",
    summary = "Get unverified papers",
    description = r#"
Retrieve a paginated list of papers that have not yet been verified against user interests.

## Overview
This endpoint returns papers from the user's RSS subscriptions that are waiting to be verified. These papers have not yet been matched against the user's defined interests.

## Query Parameters
- `page` (optional): Page number for pagination. If not provided, returns all data
- `page_size` (optional): Number of items per page. If not provided, returns all data
- `channel` (optional): Filter papers by specific channel
- `keyword` (optional): Search keyword to filter papers by title or content
- `rss_source_id` (optional): Filter papers by specific RSS source ID
- `not_match` (optional, default: "yes"): Filter papers by verification match status

## Pagination Behavior
- If neither `page` nor `page_size` is provided, returns all unverified papers
- If either `page` or `page_size` is provided, uses pagination with defaults (page=1, page_size=20)

## Returns
Returns an `UnverifiedPapersResponse` object containing:
- `pagination`: Pagination metadata
  - `page`: Current page number
  - `page_size`: Items per page
  - `total`: Total number of unverified papers
  - `total_pages`: Total number of pages
- `papers`: Array of `RssPaperDataWithDetail` objects
  - Paper details: id, title, link, description, author, pub_date
  - Source information: source_id, source details
  - Additional metadata

## Use Cases
- Display papers awaiting verification
- Show new content from RSS feeds
- Pre-verification paper review
- Batch verification preparation

## Workflow
1. User subscribes to RSS sources
2. System fetches papers from RSS feeds
3. Papers appear in this unverified list
4. User triggers verification via `/verify` endpoint
5. Verified papers move to the verified papers list
"#,
    request_body = PapersRequest,
    responses(
        (status = 200, body = UnverifiedPapersResponse, description = "Successfully retrieved unverified papers with pagination"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error"),
    ),
    tag = FEED_TAG,
)]
pub async fn unverified_papers(
    State(state): State<AppState>,
    User(user): User,
    Query(payload): Query<PapersRequest>,
) -> Result<ApiResponse<UnverifiedPapersResponse>, ApiError> {
    tracing::info!("get papers");

    // Check if pagination parameters are provided
    let use_pagination = payload.page.is_some() || payload.page_size.is_some();

    // If pagination parameters are provided, use pagination; otherwise return all data
    let (offset, limit) = if use_pagination {
        let page = payload.page.unwrap_or(1);
        let page_size = payload.page_size.unwrap_or(20);
        let offset = i32::max(page - 1, 0) * page_size;
        (Some(offset), Some(page_size))
    } else {
        (None, None)
    };

    let params = GetUnverifiedPaperIdsParams {
        offset,
        limit,
        channel: payload.channel.clone(),
        keyword: payload.keyword.clone(),
        rss_source_id: payload.rss_source_id,
        not_match: payload.not_match,
    };

    let rss_papers = RssPapersQuery::get_unverified_papers(&state.conn, user.id, params.clone())
        .await
        .context(DbErrSnafu {
            stage: "get-papers",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    let total = RssPapersQuery::count_unverified_papers(&state.conn, user.id, params)
        .await
        .context(DbErrSnafu {
            stage: "count-papers",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    // Set response based on whether pagination is used
    let pagination = if use_pagination {
        let page = payload.page.unwrap_or(1);
        let page_size = payload.page_size.unwrap_or(20);
        Pagination {
            page,
            page_size,
            total,
            total_pages: if page_size > 0 {
                total / page_size as u64
            } else {
                0
            },
        }
    } else {
        // When not using pagination, return pagination info for all data
        Pagination {
            page: 1,
            page_size: total as i32,
            total,
            total_pages: 1,
        }
    };

    Ok(ApiResponse::data(UnverifiedPapersResponse {
        pagination,
        papers: rss_papers,
    }))
}
