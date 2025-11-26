use crate::{model::base::ApiResponse, state::app_state::AppState};
use axum::extract::{Query, State};
use common::{error::api_error::*, prelude::ApiCode};
use seaorm_db::query::feed::{
    rss_papers::RssPapersQuery, user_paper_verifications::UserPaperVerificationsQuery,
};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct InternalDetailRequest {
    pub paper_id: Option<i32>,
    pub verification_id: Option<i32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InternalDetailResponse {
    pub paper: seaorm_db::query::feed::rss_papers::RssPaperDataWithDetail,
    pub verification:
        Option<seaorm_db::query::feed::user_paper_verifications::VerificationWithDetails>,
}

#[utoipa::path(
    get,
    path = "/internal/detail",
    summary = "Get detail by paper_id or verification_id",
    description = r#"
Get detail information by paper_id or verification_id.

## Parameters
- `paper_id` (optional): The ID of the paper to query
- `verification_id` (optional): The ID of the verification to query

## Behavior
- **No Authentication Required**: This endpoint does not require authentication tokens
- If both `paper_id` and `verification_id` are provided, `verification_id` takes priority
- If only `paper_id` is provided: Returns paper detail with `verification: null`
- If only `verification_id` is provided: Returns paper detail with verification information
- If neither is provided: Returns 400 Bad Request

## Returns
Returns `InternalDetailResponse` with:
- `paper`: Always present, contains `RssPaperDataWithDetail`
- `verification`: Present only when querying by `verification_id`, contains `VerificationWithDetails`
"#,
    params(
        ("paper_id" = Option<i32>, Query, description = "The ID of the paper to query"),
        ("verification_id" = Option<i32>, Query, description = "The ID of the verification to query"),
    ),
    responses(
        (status = 200, body = InternalDetailResponse, description = "Successfully retrieved detail"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Database error"),
    ),
    tag = "Internal"
)]
pub async fn get_detail(
    State(state): State<AppState>,
    Query(params): Query<InternalDetailRequest>,
) -> Result<ApiResponse<InternalDetailResponse>, ApiError> {
    tracing::info!(
        paper_id = ?params.paper_id,
        verification_id = ?params.verification_id,
        "get detail by paper_id or verification_id"
    );

    // If both are provided, verification_id takes priority
    if let Some(verification_id) = params.verification_id {
        let verification_detail =
            UserPaperVerificationsQuery::get_with_detail_by_id(&state.conn, verification_id as i64)
                .await
                .context(DbErrSnafu {
                    stage: "get-verification-detail",
                    code: ApiCode::COMMON_DATABASE_ERROR,
                })?;

        let verification_detail = verification_detail.ok_or_else(|| ApiError::NotFound {
            code: ApiCode {
                http_code: 404,
                code: 200000,
            },
        })?;

        return Ok(ApiResponse::data(InternalDetailResponse {
            paper: verification_detail.paper,
            verification: Some(verification_detail.verification),
        }));
    }

    // If only paper_id is provided
    if let Some(paper_id) = params.paper_id {
        let paper = RssPapersQuery::get_with_detail_by_id(&state.conn, paper_id as i64)
            .await
            .context(DbErrSnafu {
                stage: "get-paper-detail",
                code: ApiCode::COMMON_DATABASE_ERROR,
            })?;
        return Ok(ApiResponse::data(InternalDetailResponse {
            paper,
            verification: None,
        }));
    }

    // If neither is provided
    Err(ApiError::CustomError {
        message: "Either paper_id or verification_id must be provided".to_string(),
        code: ApiCode::COMMON_FEED_ERROR,
    })
}
