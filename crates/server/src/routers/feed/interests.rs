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
- **Incremental Update**: This endpoint performs smart incremental updates based on set operations
- **Same interests**: Existing interests that match the request are kept unchanged, but if they were previously soft-deleted (deleted_at is not null), they will be restored (deleted_at set to null)
- **New interests**: Interests in the request that don't exist in the database are created as new records
- **Removed interests**: Interests that exist in the database but are not in the request are soft-deleted (deleted_at is set to current timestamp)
- Each interest gets embedded using the configured LLM model for semantic matching

## Returns
Returns an array of `i64` IDs representing the interest records that are now active (both restored and newly created).

Example response:
```json
[101, 102, 103]
```

## Side Effects
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

## Important Notes
- This is an **incremental update** operation, not a complete replacement
- Empty array will soft-delete all existing interests
- Previously deleted interests can be restored by including them in the request
- Interest embeddings are generated using the configured LLM model
- Changes take effect immediately for new verifications

## Related Endpoints
- Use `GET /interests` to retrieve current active interests
- Trigger verification with `/verify` after updating interests
"#,
    request_body = SetInterestsRequest,
    responses(
        (status = 200, description = "Successfully set user's interests, returns array of active interest record IDs (both restored and newly created)", body = Vec<i64>),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 400, description = "Invalid request data"),
        (status = 500, description = "Database error or failed to generate embeddings"),
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
