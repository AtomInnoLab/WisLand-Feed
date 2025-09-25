use super::FEED_TAG;
use crate::model::page::{Page, Pagination};
use crate::{middlewares::auth::User, model::base::ApiResponse, state::app_state::AppState};
use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, FixedOffset, Local, TimeZone};
use common::{error::api_error::*, prelude::ApiCode};
use feed::dispatch;
use feed::redis::verify_job::{JobDetail, VerifyJob};
use feed::workers::verify_user_papers::VerifyAllUserPapersInput;
use seaorm_db::entities::feed::sea_orm_active_enums::VerificationMatch;
use seaorm_db::query::feed::user_paper_verifications::{
    ListVerifiedParams, MarkReadParams, UserPaperVerificationsQuery, VerifiedPaperItem,
};
use seaorm_db::query::feed::utils::{
    UserUnverifiedPapers, count_user_unread_papers, get_user_unverified_papers_count_info,
};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema, Clone, Copy)]
pub struct TimeRangeParam {
    pub start: Option<DateTime<FixedOffset>>,
    pub end: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FeedRequest {
    pub channel: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyRequest {
    pub channel: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AllVerifiedPapersRequest {
    #[serde(flatten)]
    pub pagination: Page,
    pub channel: Option<String>,
    pub matches: Option<Vec<VerificationMatch>>,
    pub user_interest_ids: Option<Vec<i64>>,
    pub time_range: Option<TimeRangeParam>,
    pub ignore_time_range: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct AllVerifiedPapersResponse {
    pub pagination: Pagination,
    pub papers: Vec<VerifiedPaperItem>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PapersReadRequest {
    pub paper_ids: Vec<i32>,
    pub channel: Option<String>,
    pub read_all: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeletePapersRequest {
    pub ids: Vec<i32>,
}

#[utoipa::path(
    get,
    path = "/unverified-count-info",
    responses(
        (status = 200, body = UserUnverifiedPapers),
    ),
    tag = FEED_TAG,
)]
pub async fn unverified_count_info(
    State(state): State<AppState>,
    User(user): User,
) -> Result<ApiResponse<UserUnverifiedPapers>, ApiError> {
    tracing::info!("get unverified count");

    let count_result = get_user_unverified_papers_count_info(&state.conn, user.id)
        .await
        .context(DbErrSnafu {
            stage: "count-user-unverified-papers",
            code: ApiCode::COMMON_FEED_ERROR,
        })?;

    Ok(ApiResponse::data(count_result))
}

#[utoipa::path(
    get,
    path = "/unread-count",
    request_body = FeedRequest,
    params(
        ("channel" = String, Path, description = "频道"),
    ),
    responses(
        (status = 200, body = u64),
    ),
    tag = FEED_TAG,
)]
pub async fn unread_count(
    Query(payload): Query<FeedRequest>,
    State(state): State<AppState>,
    User(user): User,
) -> Result<ApiResponse<u64>, ApiError> {
    tracing::info!("get unread count");
    let count = count_user_unread_papers(&state.conn, user.id, payload.channel)
        .await
        .context(DbErrSnafu {
            stage: "count-user-unverified-papers",
            code: ApiCode::COMMON_FEED_ERROR,
        })?;

    Ok(ApiResponse::data(count as u64))
}

#[utoipa::path(
    post,
    path = "/verify",
    request_body = VerifyRequest,
    params(
        ("channel" = String, Path, description = "频道"),
    ),
    responses(
        (status = 200, body = bool),
    ),
    tag = FEED_TAG,
)]
pub async fn verify(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<VerifyRequest>,
) -> Result<ApiResponse<bool>, ApiError> {
    tracing::info!("verify papers");

    dispatch(
        VerifyAllUserPapersInput {
            user_id: user.id,
            channel: payload.channel,
            max_prompt_number: state.config.rss.max_prompt_number,
            max_rss_paper: state.config.rss.max_rss_paper,
        },
        state.redis.apalis_conn,
    )
    .await
    .map_err(|e| ApiError::CustomError {
        message: format!("verify_papers: {e}"),
        code: ApiCode::COMMON_FEED_ERROR,
    })?;
    Ok(ApiResponse::data(true))
}

#[utoipa::path(
    get,
    path = "/verify-status",
    params(
        ("channel" = String, Path, description = "频道"),
    ),
    responses(
        (status = 200, body = JobDetail),
    ),
    tag = FEED_TAG,
)]
pub async fn verify_detail(
    Query(payload): Query<FeedRequest>,
    State(state): State<AppState>,
    User(user): User,
) -> Result<ApiResponse<Option<JobDetail>>, ApiError> {
    tracing::info!("verify papers status");
    let job = VerifyJob::new(
        state.redis.pool,
        state.config.rss.feed_redis.redis_prefix.clone(),
        user.id,
        payload.channel.as_deref(),
        state.config.rss.feed_redis.redis_key_default_expire,
    );
    let detail = job
        .get_job_detail()
        .await
        .map_err(|e| ApiError::CustomError {
            message: format!("verify_papers-detail: {e}"),
            code: ApiCode::COMMON_FEED_ERROR,
        })?;

    Ok(ApiResponse::data(detail))
}

#[utoipa::path(
    post,
    path = "/all-verified-papers",
    request_body = AllVerifiedPapersRequest,
    responses(
        (status = 200, body = AllVerifiedPapersResponse),
    ),
    tag = FEED_TAG,
)]
pub async fn all_verified_papers(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<AllVerifiedPapersRequest>,
) -> Result<ApiResponse<AllVerifiedPapersResponse>, ApiError> {
    tracing::info!("list all verified papers");

    // 处理时间范围，如果开始时间没有指定，则设置为今天的零点
    let time_range = payload.time_range.map(|tr| {
        let start = tr.start.unwrap_or_else(|| {
            // 获取今天的零点（本地时间转换为固定偏移时间）
            let today_start = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
            Local
                .from_local_datetime(&today_start)
                .unwrap()
                .fixed_offset()
        });
        (Some(start), tr.end)
    });

    let verified_papers = UserPaperVerificationsQuery::list_verified_by_user(
        &state.conn,
        user.id,
        ListVerifiedParams {
            channel: payload.channel.clone(),
            matches: payload.matches.clone(),
            user_interest_ids: payload.user_interest_ids.clone(),
            time_range,
            offset: payload.pagination.offset(),
            limit: payload.pagination.page_size(),
            ignore_time_range: payload.ignore_time_range,
        },
    )
    .await
    .context(DbErrSnafu {
        stage: "list-verified-papers",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;

    Ok(ApiResponse::data(AllVerifiedPapersResponse {
        pagination: Pagination {
            page: payload.pagination.page(),
            page_size: payload.pagination.page_size(),
            total: verified_papers.total,
            total_pages: verified_papers.total / payload.pagination.page_size() as u64,
        },
        papers: verified_papers.items,
    }))
}

#[utoipa::path(
    post,
    path = "/mark-as-read",
    request_body = MarkReadParams,
    responses(
        (status = 200, body = u64),
    ),
    tag = FEED_TAG,
)]
pub async fn papers_make_read(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<MarkReadParams>,
) -> Result<ApiResponse<u64>, ApiError> {
    tracing::info!("list all verified papers");

    let result = UserPaperVerificationsQuery::mark_read_by_user(&state.conn, user.id, payload)
        .await
        .context(DbErrSnafu {
            stage: "list-rss-sources",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    Ok(ApiResponse::data(result))
}

#[utoipa::path(
    post,
    path = "/batch-delete",
    request_body = DeletePapersRequest,
    responses(
        (status = 200, body = u64),
    ),
    tag = FEED_TAG,
)]
pub async fn batch_delete(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<DeletePapersRequest>,
) -> Result<ApiResponse<u64>, ApiError> {
    tracing::info!("delete verified papers by ids");

    let affected =
        UserPaperVerificationsQuery::delete_by_user_and_ids(&state.conn, user.id, payload.ids)
            .await
            .context(DbErrSnafu {
                stage: "delete-verified-papers",
                code: ApiCode::COMMON_DATABASE_ERROR,
            })?;

    Ok(ApiResponse::data(affected))
}
