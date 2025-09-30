use super::FEED_TAG;
use crate::model::page::{Page, Pagination};
use crate::{middlewares::auth::User, model::base::ApiResponse, state::app_state::AppState};
use axum::Json;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use chrono::{DateTime, FixedOffset, Local, TimeZone};
use common::{error::api_error::*, prelude::ApiCode};
use feed::dispatch;
use feed::redis::verify_job::{JobDetail, VerifyJob};
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
use std::convert::Infallible;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
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
            keyword: payload.keyword.clone(),
        },
    )
    .await
    .context(DbErrSnafu {
        stage: "list-verified-papers",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;

    // 查询用户兴趣与订阅源
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

// SSE 消息处理器，用于将 Redis PubSub 消息转发到 SSE 流
struct SseMessageHandler {
    user_id: String,
    channel: String,
    sender: broadcast::Sender<String>,
}

impl SseMessageHandler {
    fn new(user_id: String, channel: String, sender: broadcast::Sender<String>) -> Self {
        Self {
            user_id,
            channel,
            sender,
        }
    }
}

impl feed::redis::pubsub::MessageHandler for SseMessageHandler {
    fn event_name(&self) -> String {
        self.channel.clone()
    }

    fn handle(&self, message: String) {
        tracing::info!(
            "Received Redis message for user {}: {}",
            self.user_id,
            message
        );

        // 将 Redis 消息转发到 SSE 流
        if self.sender.send(message).is_err() {
            tracing::warn!(
                "Failed to send message to SSE stream for user {}",
                self.user_id
            );
        }
    }
}

// 连接状态监控器
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

        // 从 Redis PubSub 取消订阅
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
    get,
    path = "/stream-verify",
    responses(
        (status = 200, description = "SSE 连接测试"),
    ),
    tag = FEED_TAG,
)]
pub async fn stream_verify(
    State(state): State<AppState>,
    User(user): User,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    tracing::info!("SSE connection established for user: {}", user.id);
    let user_id = user.id;
    let channel = state.config.rss.verify_papers_channel.clone();

    // 创建连接监控器，当 SSE 流结束时会自动触发 Drop
    let monitor =
        ConnectionMonitor::new(user_id, state.redis.pubsub_manager.clone(), channel.clone());

    // 创建广播通道用于 Redis PubSub 消息转发
    let (tx, rx) = broadcast::channel::<String>(100);

    // 创建消息处理器，将 Redis 消息转发到 SSE 流
    let handler = Box::new(SseMessageHandler::new(user_id.to_string(), channel, tx));

    // 在独立的任务中启动监听器，避免阻塞
    let pubsub_manager = state.redis.pubsub_manager.clone();
    tokio::spawn(async move {
        pubsub_manager.add_listener(handler).await;
    });

    let stream = stream::unfold(
        (0, monitor, rx),
        move |(mut counter, monitor, mut receiver)| async move {
            // 检查连接是否仍然活跃
            if !monitor.is_connected() {
                tracing::info!("Connection lost for user: {}", user_id);
                return None;
            }

            // 使用 tokio::select! 同时监听定时器和 Redis 消息
            tokio::select! {
                // 定时发送心跳消息
                _ = tokio::time::sleep(Duration::from_secs(5)) => {
                    counter += 1;

                    let event_data = serde_json::json!({
                        "type": "heartbeat",
                        "message": format!("Heartbeat #{}", counter),
                        "user_id": user_id,
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                        "counter": counter,
                        "status": "connected"
                    });

                    Some((
                        Ok(Event::default()
                            .event("heartbeat")
                            .data(format!("data: {event_data}"))),
                        (counter, monitor, receiver),
                    ))
                }
                // 接收 Redis PubSub 消息
                result = receiver.recv() => {
                    match result {
                        Ok(message) => {
                            tracing::info!("Forwarding Redis message to SSE for user {}: {}", user_id, message);

                            let event_data = serde_json::json!({
                                "type": "verify_paper_success",
                                "message": message,
                                "user_id": user_id,
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                                "status": "connected"
                            });

                            Some((
                                Ok(Event::default()
                                    .event("verify_paper_success")
                                    .data(format!("data: {event_data}"))),
                                (counter, monitor, receiver),
                            ))
                        }
                        Err(_) => {
                            tracing::warn!("Redis message receiver closed for user {}", user_id);
                            // 继续运行，只是不再接收 Redis 消息
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            Some((
                                Ok(Event::default()
                                    .event("error")
                                    .data(format!("data: {}", serde_json::json!({
                                        "type": "error",
                                        "message": "Redis message channel closed",
                                        "user_id": user_id,
                                        "timestamp": chrono::Utc::now().to_rfc3339(),
                                    })))),
                                (counter, monitor, receiver),
                            ))
                        }
                    }
                }
            }
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
}
