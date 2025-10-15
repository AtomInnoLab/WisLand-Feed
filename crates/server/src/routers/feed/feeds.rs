use super::FEED_TAG;
use crate::model::page::{Page, Pagination};
use crate::{
    middlewares::auth::{User, UserInfo},
    model::base::ApiResponse,
    state::app_state::AppState,
};
use axum::Json;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use chrono::{DateTime, FixedOffset, Local, TimeZone};
use common::{error::api_error::*, prelude::ApiCode};
use feed::dispatch;
use feed::redis::pubsub::RedisPubSubManager;
use feed::redis::verify_job::{JobDetail, VerifyJob};
use feed::redis::verify_manager::VerifyManager;
use feed::workers::update_user_interest_metadata::run_update_user_interest_metadata;
use feed::workers::verify_user_papers::VerifyAllUserPapersInput;
use futures::stream::{self, Stream};
use seaorm_db::entities::feed::sea_orm_active_enums::VerificationMatch;
use seaorm_db::query::feed::user_paper_verifications::{
    ListVerifiedParams, MarkReadParams, UserPaperVerificationsQuery, VerifiedPaperItem,
};
use seaorm_db::query::feed::utils::{
    UserUnverifiedPapers, count_user_unread_papers, get_user_unverified_papers_count_info,
};
use seaorm_db::{
    entities::feed::rss_sources,
    query::feed::{
        rss_sources::RssSourcesQuery, rss_subscriptions::RssSubscriptionsQuery,
        user_interests::UserInterestsQuery,
    },
};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::signal;
use tokio::sync::broadcast;
use utoipa::ToSchema;

use feed::workers::verify_user_scheduler::{VerifyResultWithStats, has_match_yes_in_results};

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

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct AllVerifiedPapersRequest {
    #[serde(flatten)]
    pub pagination: Page,
    pub channel: Option<String>,
    pub matches: Option<Vec<VerificationMatch>>,
    pub user_interest_ids: Option<Vec<i64>>,
    pub time_range: Option<TimeRangeParam>,
    pub ignore_time_range: Option<bool>,
    pub keyword: Option<String>,
    pub rss_source_id: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct AllVerifiedPapersResponse {
    pub pagination: Pagination,
    pub papers: Vec<VerifiedPaperItem>,
    pub interest_map: HashMap<i64, String>,
    pub source_map: HashMap<i32, rss_sources::Model>,
    pub user_interest_stats: HashMap<i64, u64>,
    pub today_count: u64,
    pub yesterday_count: u64,
    pub older_than_three_days_count: u64,
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct StreamVerifyRequest {
    pub channel: Option<String>,
    pub max_match_limit_per_user: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/unverified-count-info",
    summary = "Get unverified papers count information",
    description = r#"
Retrieve the count information of unverified papers for the authenticated user.

## Overview
This endpoint returns statistics about papers that have not yet been verified against the user's interests.

## Returns
Returns a `UserUnverifiedPapers` object containing:
- Total count of unverified papers
- Breakdown by source or category
- Additional metadata about unverified papers
"#,
    responses(
        (status = 200, body = UserUnverifiedPapers, description = "Successfully retrieved unverified papers count information"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Internal server error"),
    ),
    tag = FEED_TAG,
)]
pub async fn unverified_count_info(
    State(state): State<AppState>,
    User(user): User,
) -> Result<ApiResponse<UserUnverifiedPapers>, ApiError> {
    tracing::info!("get unverified count");
    tracing::info!("user id: {:?}", user);

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
    summary = "Get unread papers count",
    description = r#"
Retrieve the total count of unread papers for the authenticated user.

## Overview
This endpoint returns the number of verified papers that the user has not yet marked as read.

## Parameters
- `channel` (optional): Filter by specific channel to get unread count for that channel only

## Returns
Returns a `u64` representing the total count of unread papers.
"#,
    request_body = FeedRequest,
    params(
        ("channel" = Option<String>, Query, description = "Optional channel filter to get unread count for specific channel"),
    ),
    responses(
        (status = 200, body = u64, description = "Successfully retrieved unread papers count"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Internal server error"),
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
    summary = "Trigger paper verification",
    description = r#"
Initiate the verification process for user's papers against their interests.

## Overview
This endpoint triggers an asynchronous verification job that matches unverified papers from the user's RSS subscriptions against their defined interests. The verification process uses AI to determine relevance.

## Process
1. Creates a verification job and adds it to the queue
2. Returns immediately with a success indicator
3. Verification runs asynchronously in the background
4. Progress can be tracked via the `/verify-status` endpoint

## Parameters
- `channel`: The channel to filter papers for verification

## Request Body
```json
{
  "channel": "default"
}
```

## Returns
Returns `true` if the verification job was successfully queued.
"#,
    request_body = VerifyRequest,
    responses(
        (status = 200, body = bool, description = "Verification job successfully queued, returns true"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Failed to queue verification job"),
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
    summary = "Get verification job status",
    description = r#"
Retrieve the current status and details of the user's paper verification job.

## Overview
This endpoint returns detailed information about an ongoing or completed verification job, including progress, counts, and any errors.

## Parameters
- `channel` (optional): Filter status by specific channel

## Returns
Returns a `JobDetail` object containing:
- Job ID and status (pending, running, completed, failed)
- Progress information (processed count, total count, percentage)
- Success and failure counts
- Token usage statistics
- Error messages if any
- Timestamps for job creation and updates

Returns `null` if no verification job exists for the user.
"#,
    params(
        ("channel" = Option<String>, Query, description = "Optional channel filter to get verification status for specific channel"),
    ),
    responses(
        (status = 200, body = Option<JobDetail>, description = "Successfully retrieved verification job details, returns null if no job exists"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Failed to retrieve verification status"),
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
    get,
    path = "/all-verified-papers",
    summary = "Get all verified papers",
    description = r#"
Retrieve a paginated list of all verified papers for the authenticated user.

## Overview
This endpoint returns papers that have been verified against the user's interests, with various filtering and pagination options.

## Query Parameters
- `page`: Page number (starts from 1)
- `page_size`: Number of items per page
- `channel` (optional): Filter by specific channel
- `matches` (optional): Filter by verification match types (Yes, No, Maybe)
- `user_interest_ids` (optional): Filter by specific interest IDs
- `time_range` (optional): Filter papers within a time range
  - `start`: Start datetime (defaults to today's midnight if not specified)
  - `end`: End datetime
- `ignore_time_range` (optional): Ignore time range filter
- `keyword` (optional): Search keyword to filter papers by title or content
- `rss_source_id` (optional): Filter papers by specific RSS source ID

## Returns
Returns an `AllVerifiedPapersResponse` object containing:
- `pagination`: Pagination information (page, page_size, total, total_pages)
- `papers`: Array of verified paper items with verification details
- `interest_map`: Mapping of interest IDs to interest names
- `source_map`: Mapping of source IDs to RSS source details
- `user_interest_stats`: Statistics of matched papers per user interest
- `today_count`: Count of papers verified today
- `yesterday_count`: Count of papers verified yesterday
- `older_than_three_days_count`: Count of papers verified more than 3 days ago
"#,
    params(
        AllVerifiedPapersRequest
    ),
    responses(
        (status = 200, body = AllVerifiedPapersResponse, description = "Successfully retrieved verified papers with pagination and metadata"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error or failed to retrieve papers"),
    ),
    tag = FEED_TAG,
)]
pub async fn all_verified_papers(
    State(state): State<AppState>,
    User(user): User,
    Query(payload): Query<AllVerifiedPapersRequest>,
) -> Result<ApiResponse<AllVerifiedPapersResponse>, ApiError> {
    tracing::info!("list all verified papers");
    tracing::info!("user: {:?}", user);

    // Process time range, if start time is not specified, set to today's midnight
    let time_range = payload.time_range.map(|tr| {
        let start = tr.start.unwrap_or_else(|| {
            // Get today's midnight (convert local time to fixed offset time)
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
            keyword: payload.keyword.clone(),
            rss_source_id: payload.rss_source_id,
            filter_by_unverified_lasted_paper_id: Some(!payload.ignore_time_range.unwrap_or(false)),
        },
    )
    .await
    .context(DbErrSnafu {
        stage: "list-verified-papers",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;

    // Query user interests and subscription sources
    let interest_items = UserInterestsQuery::list_by_user_id(&state.conn, user.id)
        .await
        .context(DbErrSnafu {
            stage: "list-user-interests",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;
    let interest_map: HashMap<i64, String> = interest_items
        .into_iter()
        .map(|m| (m.id, m.interest))
        .collect();

    let subscriptions = RssSubscriptionsQuery::list_by_user_id(&state.conn, user.id, None)
        .await
        .context(DbErrSnafu {
            stage: "get-rss-subscriptions",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;
    let mut source_ids: Vec<i32> = subscriptions.into_iter().map(|s| s.source_id).collect();
    source_ids.sort_unstable();
    source_ids.dedup();

    let sources: Vec<rss_sources::Model> = if source_ids.is_empty() {
        Vec::new()
    } else {
        RssSourcesQuery::get_by_ids(&state.conn, source_ids)
            .await
            .context(DbErrSnafu {
                stage: "get-rss-sources",
                code: ApiCode::COMMON_DATABASE_ERROR,
            })?
    };
    let source_map: HashMap<i32, rss_sources::Model> =
        sources.into_iter().map(|m| (m.id, m)).collect();

    Ok(ApiResponse::data(AllVerifiedPapersResponse {
        pagination: Pagination {
            page: payload.pagination.page(),
            page_size: payload.pagination.page_size(),
            total: verified_papers.total,
            total_pages: verified_papers.total / payload.pagination.page_size() as u64,
        },
        papers: verified_papers.items,
        interest_map,
        source_map,
        user_interest_stats: verified_papers.user_interest_stats,
        today_count: verified_papers.today_count,
        yesterday_count: verified_papers.yesterday_count,
        older_than_three_days_count: verified_papers.older_than_three_days_count,
    }))
}

#[utoipa::path(
    post,
    path = "/mark-as-read",
    summary = "Mark papers as read",
    description = r#"
Mark one or more verified papers as read for the authenticated user.

## Overview
This endpoint allows users to mark papers as read, which updates their read status in the database.

## Request Body
The `MarkReadParams` should contain:
- `paper_ids`: Array of paper IDs to mark as read
- `channel` (optional): Channel filter
- `read_all` (optional): Boolean flag to mark all papers as read

## Returns
Returns a `u64` representing the number of papers successfully marked as read.
"#,
    request_body = MarkReadParams,
    responses(
        (status = 200, body = u64, description = "Successfully marked papers as read, returns count of affected papers"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error or failed to mark papers as read"),
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
    summary = "Batch delete verified papers",
    description = r#"
Delete multiple verified papers by their IDs for the authenticated user.

## Overview
This endpoint allows users to permanently delete verified papers from their feed.

## Request Body
```json
{
  "ids": [1, 2, 3, 4, 5]
}
```

## Parameters
- `ids`: Array of paper IDs to delete

## Returns
Returns a `u64` representing the number of papers successfully deleted.

## Note
This operation is permanent and cannot be undone. Deleted papers will not appear in the user's feed again.
"#,
    request_body = DeletePapersRequest,
    responses(
        (status = 200, body = u64, description = "Successfully deleted papers, returns count of deleted papers"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error or failed to delete papers"),
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

// SSE message handler for forwarding Redis PubSub messages to SSE stream
struct SseMessageHandler {
    user_id: i64,
    channel: String,
    sender: broadcast::Sender<String>,
}

impl SseMessageHandler {
    fn new(user_id: i64, channel: String, sender: broadcast::Sender<String>) -> Self {
        Self {
            user_id,
            channel,
            sender,
        }
    }
}

impl feed::redis::pubsub::MessageHandler for SseMessageHandler {
    fn event_name(&self) -> String {
        RedisPubSubManager::build_user_channel(&self.channel, self.user_id)
    }

    fn handle(&self, message: String) {
        tracing::info!("Received Redis message for user {}", self.user_id);

        // Parse message to check if there's at least one "yes" match
        let result_with_stats: VerifyResultWithStats = match serde_json::from_str(&message) {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!(
                    "Failed to parse Redis message as VerifyResultWithStats: {}",
                    e
                );
                tracing::debug!("Raw message that failed to parse: {}", message);
                // If parsing fails, forward anyway to maintain backward compatibility
                if self.sender.send(message).is_err() {
                    tracing::warn!(
                        "Failed to send message to SSE stream for user {}",
                        self.user_id
                    );
                }
                return;
            }
        };

        // Check if verification_details exists and has at least one "yes" match
        let has_yes_match = has_match_yes_in_results(&result_with_stats.verification_details);

        tracing::debug!(
            "Paper has {} verifications, has_yes_match: {}",
            result_with_stats
                .verification_details
                .as_ref()
                .map(|v| v.verifications.len())
                .unwrap_or(0),
            has_yes_match
        );

        // Only forward message if there is at least one "yes" match
        if !has_yes_match {
            tracing::info!(
                "No 'yes' match found in verifications, skipping message for user {}",
                self.user_id
            );
            return;
        }

        tracing::info!(
            "Forwarding message with 'yes' match to SSE stream for user {}",
            self.user_id
        );

        // Forward Redis message to SSE stream
        if self.sender.send(message).is_err() {
            tracing::warn!(
                "Failed to send message to SSE stream for user {}",
                self.user_id
            );
        }
    }
}

// Connection status monitor
struct ConnectionMonitor {
    user_id: i64,
    is_connected: Arc<AtomicBool>,
    pubsub_manager: feed::redis::pubsub::RedisPubSubManager,
    channel: String,
}

impl ConnectionMonitor {
    fn new(
        user_id: i64,
        pubsub_manager: feed::redis::pubsub::RedisPubSubManager,
        channel: String,
    ) -> Self {
        Self {
            user_id,
            is_connected: Arc::new(AtomicBool::new(true)),
            pubsub_manager,
            channel,
        }
    }

    fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Relaxed)
    }
}

impl Drop for ConnectionMonitor {
    fn drop(&mut self) {
        self.is_connected.store(false, Ordering::Relaxed);
        tracing::info!(
            "SSE connection dropped for user: {}, performing cleanup...",
            self.user_id
        );

        // Unsubscribe from Redis PubSub
        let pubsub_manager = self.pubsub_manager.clone();
        let channel = self.channel.clone();
        let user_id = self.user_id;

        tokio::spawn(async move {
            if let Err(e) = pubsub_manager.unsubscribe(&channel).await {
                tracing::error!(
                    "Failed to unsubscribe from Redis channel '{}' for user {}: {}",
                    channel,
                    user_id,
                    e
                );
            } else {
                tracing::info!(
                    "Successfully unsubscribed from Redis channel '{}' for user {}",
                    channel,
                    user_id
                );
            }
        });

        tracing::info!("Cleanup completed for user: {}", self.user_id);
    }
}

#[utoipa::path(
    post,
    path = "/stream-verify",
    summary = "Stream verification progress via SSE",
    description = r#"
Establish a Server-Sent Events (SSE) connection to receive real-time updates during paper verification.

## Overview
This endpoint creates a persistent SSE connection that streams verification progress updates to the client in real-time. It's useful for showing live progress in the UI.

## Request Body
```json
{
  "channel": "arxiv",
  "max_match_limit_per_user": 50
}
```

## Parameters
- `channel` (optional): Channel to filter verification updates
- `max_match_limit_per_user` (optional): Maximum number of papers to match per user. When this limit is reached, a `match_limit_reached` event will be sent and the connection will be closed

## SSE Event Types
The stream emits the following event types:

1. **heartbeat**: Periodic status updates every 5 seconds
   - Contains: user_id, verify_info, timestamp, status, is_completed
   
2. **verify_paper_success**: Sent when a paper is successfully verified (only for papers with at least one "Yes" match)
   - Contains: verification_details (paper info and verification results), user_verify_info (user statistics), timestamp, status
   
3. **verify_completed**: Sent when all papers have been verified
   - Contains: timestamp, status, is_completed flag
   - The connection will be closed after sending this event

4. **match_limit_reached**: Sent when the matched paper count reaches the maximum limit
   - Contains: user_id, matched, max_limit, timestamp, status
   - The connection will be closed after sending this event

## Event Data Structure

### heartbeat event
```json
{
  "type": "heartbeat",
  "user_id": 123,
  "verify_info": {
    "pending_unverify_count": 10,
    "success_count": 5,
    "fail_count": 1,
    "processing_count": 2,
    "total": 18,
    "token_usage": 1500,
    "matched_count": 8,
    "max_match_limit": 50,
    "total_matched_count": 8
  },
  "timestamp": "2024-01-01T12:00:00Z",
  "status": "connected",
  "is_completed": false
}
```

### verify_paper_success event
```json
{
  "type": "verify_paper_success",
  "verification_details": {
    "paper": {
      "id": 456,
      "title": "Example Paper Title",
      "arxiv_id": "2401.12345",
      "abstract": "Paper abstract...",
      "published_at": "2024-01-01T00:00:00Z",
      // ... other paper fields
    },
    "verifications": [
      {
        "id": 789,
        "user_id": 123,
        "paper_id": 456,
        "user_interest": {
          "id": 1,
          "interest": "Machine Learning",
          // ... other interest fields
        },
        "match": "Yes",
        "relevance_score": 0.95,
        "verified_at": "2024-01-01T12:00:00Z",
        "channel": "arxiv",
        "metadata": null,
        "unread": true,
        "created_at": "2024-01-01T12:00:00Z",
        "updated_at": "2024-01-01T12:00:00Z"
      }
    ]
  },
  "user_verify_info": {
    "pending_unverify_count": 9,
    "success_count": 6,
    "fail_count": 1,
    "processing_count": 2,
    "total": 18,
    "token_usage": 1600,
    "matched_count": 9,
    "max_match_limit": 50,
    "total_matched_count": 9
  },
  "timestamp": "2024-01-01T12:00:00Z",
  "status": "connected",
  "is_completed": false
}
```

### match_limit_reached event
```json
{
  "type": "match_limit_reached",
  "user_id": 123,
  "matched": 50,
  "max_limit": 50,
  "timestamp": "2024-01-01T12:05:00Z",
  "status": "limit_reached"
}
```

## Connection Management
- Connection automatically updates user interest metadata before starting
- Subscribes to Redis pub/sub for real-time updates
- Automatically unsubscribes and cleans up when connection is closed
- Sends keep-alive messages every 10 seconds
- Only forwards papers with at least one "Yes" match
- Monitors matched paper count and disconnects when reaching the specified limit (if provided)
  - When matched_count >= max_match_limit_per_user, a `match_limit_reached` event is sent
  - The connection is then closed to prevent further processing

## Note
This is a long-lived connection. The client should be prepared to handle connection drops and reconnect if needed. The connection may be terminated early if the maximum match limit is reached.
"#,
    request_body = StreamVerifyRequest,
    responses(
        (status = 200, description = "SSE connection established successfully, will stream verification updates"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Failed to establish SSE connection or update metadata"),
    ),
    tag = FEED_TAG,
)]
pub async fn stream_verify(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<StreamVerifyRequest>,
) -> Sse<Pin<Box<dyn Stream<Item = Result<Event, ApiError>> + Send>>> {
    tracing::info!("SSE connection established for user: {}", user.id);
    let user_id = user.id;
    let verify_papers_sub_channel = state.config.rss.verify_papers_channel.clone();

    let result = run_update_user_interest_metadata(
        Some(user_id.to_string()),
        state.config.clone(),
        state.conn.clone(),
        state.redis.pool.clone(),
    )
    .await;
    if result.is_err() {
        tracing::error!(
            "Failed to update user interest metadata: {}",
            result.err().unwrap()
        );
        // Return error stream
        return Sse::new(Box::pin(stream::once(async {
            Err(ApiError::CustomError {
                message: "Failed to update user interest metadata".to_string(),
                code: ApiCode::COMMON_FEED_ERROR,
            })
        })));
    } else {
        tracing::info!("Successfully updated user interest metadata");
    }

    // Create connection monitor, automatically triggers Drop when SSE stream ends
    let monitor = ConnectionMonitor::new(
        user_id,
        state.redis.pubsub_manager.clone(),
        verify_papers_sub_channel.clone(),
    );

    // Create broadcast channel for Redis PubSub message forwarding
    let (tx, rx) = broadcast::channel::<String>(100);

    // Create message handler to forward Redis messages to SSE stream
    let handler = Box::new(SseMessageHandler::new(
        user_id,
        verify_papers_sub_channel,
        tx,
    ));

    // Start listener in separate task to avoid blocking
    let pubsub_manager = state.redis.pubsub_manager.clone();
    tokio::spawn(async move {
        pubsub_manager.add_listener(handler).await;
    });

    let verify_manager = VerifyManager::new(
        state.redis.clone().pool,
        state.conn.clone(),
        state.config.rss.feed_redis.redis_prefix.clone(),
        state.config.rss.feed_redis.redis_key_default_expire,
    )
    .await;

    let stream = stream::unfold(
        (monitor, rx, verify_manager.clone(), false), // Add completion flag
        move |(monitor, mut receiver, verify_manager_clone, mut is_completed)| async move {
            // Check if connection is still active or already completed
            if !monitor.is_connected() || is_completed {
                tracing::info!("Ending SSE stream for user: {}", user_id);
                return None;
            }

            // Check verification status before waiting
            let verify_info = verify_manager_clone
                .get_user_unverified_info(user_id)
                .await
                .unwrap();
            let verification_completed = verify_info.total == 0
                || (verify_info.success_count + verify_info.fail_count) == verify_info.total;

            if verification_completed {
                // If completed, send completion event and mark as completed
                tracing::info!(
                    "Verification completed for user {}, sending completion event",
                    user_id
                );

                let completion_event_data = serde_json::json!({
                    "type": "verify_completed",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "status": "completed",
                    "is_completed": true,
                });

                let completion_event: Result<Event, ApiError> = Ok(Event::default()
                    .event("verify_completed")
                    .data(format!("data: {completion_event_data}")));

                is_completed = true;
                return Some((
                    completion_event,
                    (monitor, receiver, verify_manager_clone, is_completed),
                ));
            }

            // Use tokio::select! to simultaneously listen to timer, Redis messages and shutdown signal
            tokio::select! {
                // Listen for shutdown signal
                _ = signal::ctrl_c() => {
                    tracing::info!("SSE stream received shutdown signal for user: {}", user_id);
                    None
                }
                // Send heartbeat message periodically
                _ = tokio::time::sleep(Duration::from_secs(5)) => {
                    // Get current verify info for heartbeat
                    let verify_info = match verify_manager_clone.get_user_unverified_info(user_id).await {
                        Ok(info) => info,
                        Err(e) => {
                            tracing::error!("Failed to get verify info for heartbeat: {}", e);
                            return None;
                        }
                    };

                    // Send normal heartbeat (completion already checked before select)
                    let event_data = serde_json::json!({
                        "type": "heartbeat",
                        "user_id": user_id,
                        "verify_info": verify_info,
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                        "status": "connected",
                        "is_completed": false,
                    });

                    let event: Result<Event, ApiError> = Ok(Event::default()
                        .event("heartbeat")
                        .data(format!("data: {event_data}")));

                    Some((event, (monitor, receiver, verify_manager_clone, is_completed)))
                }
                // Receive Redis PubSub messages
                result = receiver.recv() => {
                    match result {
                        Ok(message) => {
                            tracing::info!("Forwarding Redis message to SSE for user {}", user_id);

                            // Check if match limit has been reached
                            let limit_reached = match verify_manager_clone.get_user_unverified_info(user_id).await {
                                Ok(verify_info) => {
                                    // Check if matched >= max_limit
                                    if verify_info.max_match_limit > 0 && verify_info.matched_count >= verify_info.max_match_limit {
                                        Some((verify_info.matched_count, verify_info.max_match_limit))
                                    } else {
                                        None
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to get user unverified info for user {}: {}", user_id, e);
                                    None
                                }
                            };

                            // If limit reached, send event and disconnect
                            if let Some((matched, max_limit)) = limit_reached {
                                tracing::info!(
                                    "Match limit reached for user {}: matched={}, max_limit={}. Sending match_limit_reached event and disconnecting SSE stream.",
                                    user_id,
                                    matched,
                                    max_limit
                                );

                                // Send match_limit_reached event before disconnecting
                                let limit_event_data = serde_json::json!({
                                    "type": "match_limit_reached",
                                    "user_id": user_id,
                                    "matched": matched,
                                    "max_limit": max_limit,
                                    "timestamp": chrono::Utc::now().to_rfc3339(),
                                    "status": "limit_reached",
                                });

                                let limit_event: Result<Event, ApiError> = Ok(Event::default()
                                    .event("match_limit_reached")
                                    .data(format!("data: {limit_event_data}")));

                                // Return the event and set is_completed to true to trigger disconnection on next iteration
                                is_completed = true;
                                return Some((limit_event, (monitor, receiver, verify_manager_clone, is_completed)));
                            }

                            // Parse the message as VerifyResultWithStats
                            let result_with_stats: VerifyResultWithStats = match serde_json::from_str(&message) {
                                Ok(value) => value,
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to parse message as VerifyResultWithStats: {}. Skipping message.",
                                        e
                                    );
                                    // Skip this message and continue to next iteration
                                    return Some((
                                        Ok(Event::default().event("error").data("data: {\"type\":\"error\",\"message\":\"Failed to parse message\"}")),
                                        (monitor, receiver, verify_manager_clone, is_completed)
                                    ));
                                }
                            };

                            // Always send verify_paper_success message with the complete result_with_stats
                            let event_data = serde_json::json!({
                                "type": "verify_paper_success",
                                "verification_details": result_with_stats.verification_details,
                                "user_verify_info": result_with_stats.user_verify_info,
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                                "status": "connected",
                                "is_completed": false,
                            });

                            let event: Result<Event, ApiError> = Ok(Event::default()
                                .event("verify_paper_success")
                                .data(format!("data: {event_data}")));

                            Some((event, (monitor, receiver, verify_manager_clone, is_completed)))
                        }
                        Err(_) => {
                            tracing::warn!("Redis message receiver closed for user {}", user_id);
                            // End SSE stream (trigger Drop cleanup and unsubscribe), avoid infinite loop and blocking graceful shutdown
                            None
                        }
                    }
                }
            }
        },
    );

    if let Err(e) = verify_manager
        .append_user_to_verify_list(
            user_id,
            Some(state.config.rss.max_rss_paper as i32),
            payload.channel,
            payload
                .max_match_limit_per_user
                .unwrap_or(state.config.rss.max_match_limit_per_user as i32),
        )
        .await
    {
        tracing::error!("Failed to append user to verify list: {}", e);
    }

    Sse::new(Box::pin(stream) as Pin<Box<dyn Stream<Item = Result<Event, ApiError>> + Send>>)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserVerifyInfoItem {
    pub user_id: i64,
    pub pending_unverify_count: i64,
    pub success_count: i64,
    pub fail_count: i64,
    pub processing_count: i64,
    pub total: i64,
    pub token_usage: i64,
    pub matched_count: i64,
    pub max_match_limit: i64,
    pub total_matched_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_info: Option<UserInfo>,
}

#[utoipa::path(
    get,
    path = "/all-users-verify-info",
    summary = "Get verification info for all users",
    description = r#"
Retrieve verification status information for all users currently in the verification queue.

## Overview
This endpoint returns a list of all users who have ongoing or pending verification jobs, along with their progress and statistics.

## Returns
Returns an array of `UserVerifyInfoItem` objects, each containing:
- `user_id`: The user's unique identifier
- `pending_unverify_count`: Number of papers waiting to be verified
- `success_count`: Number of papers successfully verified
- `fail_count`: Number of papers that failed verification
- `processing_count`: Number of papers currently being processed
- `total`: Total number of papers in the verification job
- `token_usage`: Total tokens consumed for this user's verification
- `matched_count`: Number of papers that matched the criteria
- `max_match_limit`: Maximum number of matches allowed
- `user_info` (optional): Detailed user information (only included for the authenticated user)

## Use Cases
- Admin dashboard to monitor all verification jobs
- System health monitoring
- Resource usage tracking

## Note
Only the authenticated user's entry will include the `user_info` field for privacy reasons.
"#,
    responses(
        (status = 200, body = Vec<UserVerifyInfoItem>, description = "Successfully retrieved verification info for all users in the queue"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Failed to retrieve verification information"),
    ),
    tag = FEED_TAG,
)]
pub async fn all_users_verify_info(
    State(state): State<AppState>,
    User(user): User,
) -> Result<ApiResponse<Vec<UserVerifyInfoItem>>, ApiError> {
    tracing::info!(
        "Getting verify info for all users, current user: {}",
        user.id
    );
    tracing::info!("user id: {:?}", user);

    let verify_manager = VerifyManager::new(
        state.redis.clone().pool,
        state.conn.clone(),
        state.config.rss.feed_redis.redis_prefix.clone(),
        state.config.rss.feed_redis.redis_key_default_expire,
    )
    .await;

    // Get all user IDs from verify list
    let user_ids = verify_manager.get_user_verify_list().await?;

    tracing::info!("Found {} users in verify list", user_ids.len());

    // Get verify info for each user
    let mut results = Vec::new();
    for user_id in user_ids {
        match verify_manager.get_user_unverified_info(user_id).await {
            Ok(info) => {
                // If this is the current user, include user info
                let user_info = if user_id == user.id {
                    Some(user.clone())
                } else {
                    None
                };

                results.push(UserVerifyInfoItem {
                    user_id,
                    pending_unverify_count: info.pending_unverify_count,
                    success_count: info.success_count,
                    fail_count: info.fail_count,
                    processing_count: info.processing_count,
                    total: info.total,
                    token_usage: info.token_usage,
                    matched_count: info.matched_count,
                    max_match_limit: info.max_match_limit,
                    total_matched_count: info.total_matched_count,
                    user_info,
                });
            }
            Err(e) => {
                tracing::warn!("Failed to get verify info for user {}: {}", user_id, e);
            }
        }
    }

    Ok(ApiResponse::data(results))
}
