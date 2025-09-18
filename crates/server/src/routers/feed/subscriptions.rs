use axum::Json;
use axum::extract::{Path, State};
use common::{error::api_error::*, prelude::ApiCode};
use seaorm_db::{
    entities::web::feed::rss_subscriptions, query::feed::rss_subscriptions::RssSubscriptionsQuery,
};
use serde::Deserialize;
use snafu::ResultExt;
use utoipa::ToSchema;

use crate::{
    middlewares::auth::User, model::base::ApiResponse, routers::feed::FEED_TAG,
    state::app_state::AppState,
};

#[utoipa::path(
    get,
    path = "/subscriptions",
    responses(
        (status = 200, body = rss_subscriptions::Model),
    ),
    tag = FEED_TAG,
)]
pub async fn subscriptions(
    State(state): State<AppState>,
    User(user): User,
) -> Result<ApiResponse<Vec<rss_subscriptions::Model>>, ApiError> {
    tracing::info!("get subscriptions");

    let subscriptions = RssSubscriptionsQuery::list_by_user_id(&state.conn, user.id, None)
        .await
        .context(DbErrSnafu {
            stage: "get-rss-subscriptions",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    Ok(ApiResponse::data(subscriptions))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SubscriptionsCreateRequest {
    pub source_ids: Vec<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SubscriptionCreateOneRequest {
    pub source_id: i32,
}

#[utoipa::path(
    post,
    path = "/subscriptions",
    request_body = SubscriptionsCreateRequest,
    responses(
        (status = 200, description = "批量订阅成功，返回新增或已存在的订阅ID列表", body = Vec<i64>),
    ),
    tag = FEED_TAG,
)]
pub async fn batch_subscriptions(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<SubscriptionsCreateRequest>,
) -> Result<ApiResponse<Vec<i64>>, ApiError> {
    tracing::info!(
        user_id = user.id,
        count = payload.source_ids.len(),
        "create subscriptions"
    );

    let ids = RssSubscriptionsQuery::replace_many(&state.conn, user.id, payload.source_ids)
        .await
        .context(DbErrSnafu {
            stage: "create-rss-subscriptions",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    Ok(ApiResponse::data(ids))
}

#[utoipa::path(
    post,
    path = "/subscriptions/one",
    request_body = SubscriptionCreateOneRequest,
    responses(
        (status = 200, description = "新增单个订阅，返回订阅ID（若已存在或源不存在则为空）", body = Option<i64>),
    ),
    tag = FEED_TAG,
)]
pub async fn subscriptions_create_one(
    State(state): State<AppState>,
    User(user): User,
    Json(body): Json<SubscriptionCreateOneRequest>,
) -> Result<ApiResponse<Option<i64>>, ApiError> {
    tracing::info!(
        user_id = user.id,
        source_id = body.source_id,
        "create one subscription"
    );

    let id = RssSubscriptionsQuery::insert_one_source(&state.conn, user.id, body.source_id)
        .await
        .context(DbErrSnafu {
            stage: "create-one-rss-subscription",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    Ok(ApiResponse::data(id))
}

#[utoipa::path(
    delete,
    path = "/subscriptions/{subscription_id}",
    params(
        ("subscription_id" = i64, Path, description = "订阅记录ID"),
    ),
    responses(
        (status = 200, description = "删除单个订阅", body = bool),
    ),
    tag = FEED_TAG,
)]
pub async fn subscriptions_delete_one(
    State(state): State<AppState>,
    User(user): User,
    Path(subscription_id): Path<i64>,
) -> Result<ApiResponse<bool>, ApiError> {
    tracing::info!(
        user_id = user.id,
        subscription_id,
        "delete one subscription"
    );

    RssSubscriptionsQuery::delete_by_id(&state.conn, subscription_id)
        .await
        .context(DbErrSnafu {
            stage: "delete-one-rss-subscription",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    Ok(ApiResponse::data(true))
}
