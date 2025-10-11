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
    pub keyword: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct AllVerifiedPapersResponse {
    pub pagination: Pagination,
    pub papers: Vec<VerifiedPaperItem>,
    pub interest_map: HashMap<i64, String>,
    pub source_map: HashMap<i32, rss_sources::Model>,
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
        ("channel" = String, Path, description = "Channel"),
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
        ("channel" = String, Path, description = "Channel"),
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
        ("channel" = String, Path, description = "Channel"),
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
    request_body = StreamVerifyRequest,
    responses(
        (status = 200, description = "SSE connection test"),
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

                            let verify_info = verify_manager_clone
                                .get_user_unverified_info(user_id)
                                .await
                                .unwrap();
                            // Try to parse Redis message as JSON, if failed treat as string
                            let parsed_message = serde_json::from_str::<serde_json::Value>(&message)
                                .unwrap_or_else(|_| serde_json::Value::String(message.clone()));

                            // Always send verify_paper_success message
                            let event_data = serde_json::json!({
                                "type": "verify_paper_success",
                                "message": parsed_message,
                                "verify_info": verify_info,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_info: Option<UserInfo>,
}

#[utoipa::path(
    get,
    path = "/all-users-verify-info",
    responses(
        (status = 200, body = Vec<UserVerifyInfoItem>),
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
