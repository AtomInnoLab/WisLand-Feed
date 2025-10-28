use axum::Json;
use axum::extract::State;
use common::{error::api_error::*, prelude::ApiCode};
use conf::config::app_config;
use feed::redis::user_interests_manager::UserInterestsManager;
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
    summary = "Get user's interests",
    description = r#"
Retrieve all interests (preferences) defined by the authenticated user.

## Overview
This endpoint returns a list of interest keywords that the user has set up for paper verification. These interests are used to match incoming RSS papers against the user's research focus.

## Returns
Returns an array of strings, each representing an interest/preference keyword.

Example response:
```json
[
  "machine learning",
  "natural language processing",
  "computer vision",
  "deep learning"
]
```

## Use Cases
- Display user's current interests
- Edit interest list UI
- Show what topics the user is tracking
- Verification configuration

## Related Endpoints
- Use `POST /interests` to update the interest list
- Interests are used in paper verification via `/verify`
"#,
    responses(
        (status = 200, description = "Successfully retrieved user's interests as an array of strings", body = Vec<String>),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error"),
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
    summary = "Set user's interests",
    description = r#"
Update the user's interest list using incremental update logic.

## Overview
This endpoint allows users to define or update their research interests. The system will use these interests to verify and match incoming RSS papers.

## Request Body
```json
{
  "interests": [
    "machine learning",
    "natural language processing",
    "computer vision"
  ]
}
```

## Parameters
- `interests`: Array of interest keywords/phrases

## Behavior
- **Asynchronous Processing**: This endpoint uses delayed execution with version control
- **High-frequency Optimization**: Multiple rapid requests are merged, only the latest request is executed
- **Incremental Update**: Performs smart incremental updates based on set operations
- **Same interests**: Existing interests that match the request are kept unchanged, but if they were previously soft-deleted (deleted_at is not null), they will be restored (deleted_at set to null)
- **New interests**: Interests in the request that don't exist in the database are created as new records
- **Removed interests**: Interests that exist in the database but are not in the request are soft-deleted (deleted_at is set to current timestamp)
- Each interest gets embedded using the configured LLM model for semantic matching

## Returns
Returns a request ID string for tracking the asynchronous operation. The actual database update happens after a 500ms delay, ensuring only the latest request is processed.

Example response:
```json
"550e8400-e29b-41d4-a716-446655440000"
```

## Side Effects
- Request is queued for asynchronous processing with 500ms delay
- Only the latest request per user will be executed (older requests are cancelled)
- Existing interests are preserved and restored if previously deleted
- New interest records are created with embeddings
- Removed interests are soft-deleted (not permanently removed)
- Interest metadata may be updated for verification purposes

## Use Cases
- Initial interest setup for new users
- Update research focus areas
- Refine paper matching criteria
- Change verification preferences
- Restore previously deleted interests
- High-frequency UI updates (typing in input fields)

## Important Notes
- This is an **asynchronous operation** with eventual consistency
- **High-frequency calls are optimized**: Multiple rapid requests are merged automatically
- Empty array will soft-delete all existing interests
- Previously deleted interests can be restored by including them in the request
- Interest embeddings are generated using the configured LLM model
- Changes take effect after the 500ms delay for new verifications
- Only the most recent request per user will be processed

## Related Endpoints
- Use `GET /interests` to retrieve current active interests
- Trigger verification with `/verify` after updating interests
"#,
    request_body = SetInterestsRequest,
    responses(
        (status = 200, description = "Successfully queued user's interests update, returns request ID for tracking", body = String),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 400, description = "Invalid request data"),
        (status = 500, description = "Failed to queue update request"),
    ),
    tag = FEED_TAG,
)]
pub async fn set_interests(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<SetInterestsRequest>,
) -> Result<ApiResponse<String>, ApiError> {
    tracing::info!(
        user_id = user.id,
        count = payload.interests.len(),
        "set interests (async)"
    );

    let config = app_config();

    // 创建 UserInterestsManager
    let manager = UserInterestsManager::new(
        state.redis.pool.clone(),
        state.config.rss.feed_redis.redis_prefix.clone(),
        state.config.rss.feed_redis.redis_key_default_expire,
    );

    // 提交延迟任务
    let request_id = manager
        .submit_update(
            user.id,
            payload.interests,
            config.llm.model.clone(),
            state.redis.apalis_conn.clone(),
        )
        .await
        .map_err(|e| ApiError::CustomError {
            message: format!("Failed to submit user interests update: {e}"),
            code: ApiCode::COMMON_FEED_ERROR,
        })?;

    tracing::info!(
        user_id = user.id,
        request_id = %request_id,
        "Successfully queued user interests update"
    );

    // 立即返回 request_id（不等待数据库操作）
    Ok(ApiResponse::data(request_id))
}
