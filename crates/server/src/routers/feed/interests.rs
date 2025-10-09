use axum::Json;
use axum::extract::State;
use common::{error::api_error::*, prelude::ApiCode};
use conf::config::app_config;
use seaorm_db::query::feed::user_interests::UserInterestsQuery;
use serde::Deserialize;
use snafu::ResultExt;
use utoipa::ToSchema;

use crate::{
    middlewares::auth::User, model::base::ApiResponse, routers::feed::FEED_TAG,
    state::app_state::AppState,
};


#[utoipa::path(
    get,
    path = "/interests",
    responses(
        (status = 200, description = "获取当前用户偏好列表，仅返回兴趣名称", body = Vec<String>),
    ),
    tag = FEED_TAG,
)]
pub async fn interests(
    State(state): State<AppState>,
    User(user): User,
) -> Result<ApiResponse<Vec<String>>, ApiError> {
    tracing::info!(user_id = user.id, "list interests");

    let items = UserInterestsQuery::list_by_user_id(&state.conn, user.id)
        .await
        .context(DbErrSnafu {
            stage: "list-user-interests",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    let interests = items.into_iter().map(|m| m.interest).collect();
    Ok(ApiResponse::data(interests))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetInterestsRequest {
    pub interests: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/interests",
    request_body = SetInterestsRequest,
    responses(
        (status = 200, description = "为当前用户设置多个偏好，返回新建记录ID列表", body = Vec<i64>),
    ),
    tag = FEED_TAG,
)]
pub async fn set_interests(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<SetInterestsRequest>,
) -> Result<ApiResponse<Vec<i64>>, ApiError> {
    tracing::info!(
        user_id = user.id,
        count = payload.interests.len(),
        "set interests"
    );

    let config = app_config();

    let ids = UserInterestsQuery::replace_many(
        &state.conn,
        user.id,
        payload.interests,
        config.llm.model.clone(),
    )
    .await
    .context(DbErrSnafu {
        stage: "insert-user-interests",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;

    // dispatch(
    //     UpdateUserInterestMetadataInputOnce {
    //         user_id: Some(user.id.to_string()),
    //     },
    //     state.redis.apalis_conn,
    // )
    // .await
    // .map_err(|e| ApiError::FeedError {
    //     message: format!("update_user_interest_metadata_once: {e}"),
    //     stage: "update_user_interest_metadata_once".to_string(),
    //     code: ApiCode::COMMON_FEED_ERROR,
    // })?;

    Ok(ApiResponse::data(ids))
}
