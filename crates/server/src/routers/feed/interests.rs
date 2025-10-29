use axum::Json;
use axum::extract::State;
use common::{error::api_error::*, prelude::ApiCode};
use conf::config::app_config;
use feed::redis::update_task_manager::{
    TaskType, UpdateTaskData, UpdateTaskInput, UpdateTaskManager,
};
use seaorm_db::query::feed::user_interests::UserInterestsQuery;
use serde::Deserialize;
use snafu::ResultExt;
use utoipa::ToSchema;
use uuid::Uuid;

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
Update the user's interest list using advanced incremental update logic with optimization for high-frequency updates.

## Overview
This endpoint allows users to define or update their research interests. The system uses these interests to verify and match incoming RSS papers against user's research focus areas using AI-powered semantic similarity.

## Request Body

```json
{
  "interests": [
    "machine learning",
    "natural language processing",
    "computer vision",
    "deep learning"
  ]
}
```

### Parameters
- `interests` (required): Array of interest keywords/phrases. Each interest is a string representing a research topic or keyword. Empty array `[]` is allowed and will clear all interests.

### Validation & Limits
- **Maximum Count**: The number of interests is limited by `rss.max_prompt_number` configuration (default: 10). Requests exceeding this limit will return a 400 error with a descriptive message.
- If the request contains more interests than allowed, it will be rejected before queuing.

## Behavior & Update Logic

### Asynchronous Processing with 500ms Delay
This endpoint uses a sophisticated delayed execution mechanism:
- **Request Queuing**: Requests are queued for processing
- **500ms Delay**: There's a 500ms delay before actual database operations
- **Latest-Wins Strategy**: Only the most recent request per user is executed
- **Auto-Cancel**: Older requests are automatically cancelled if a new request arrives within the delay period

### Incremental Update Strategy
The update is smart and performs set-based operations:

#### 1. **Preserved Interests** (Unchanged)
- Existing interests that appear in the new request are kept as-is
- If they were previously soft-deleted (`deleted_at` was set), they are restored (`deleted_at` set to `null`)
- No database changes for these interests

#### 2. **New Interests** (Created)
- Interests in the request that don't exist in the database are created as new records
- Each new interest gets an LLM-generated embedding for semantic matching
- Created with current timestamp and marked as active

#### 3. **Removed Interests** (Soft-Deleted)
- Interests in the database that are NOT in the new request are soft-deleted
- `deleted_at` timestamp is set to current time
- Records are NOT permanently deleted, can be restored by re-adding them

### High-Frequency Optimization
The system is optimized for rapid, repeated updates (e.g., user typing in input field):
- Multiple requests within 500ms are automatically merged
- Only the final state is applied to the database
- Reduces database load and prevents race conditions
- Makes the UI responsive to user typing

## Returns

Returns a request ID (UUID string) for tracking the asynchronous operation:

```json
{
  "success": true,
  "message": "Success",
  "data": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Important**: This does NOT mean the database update has completed. The actual update happens ~500ms later.

## Side Effects & Processing

### Database Operations (After 500ms Delay)
1. **Restore soft-deleted interests** that match the request
2. **Create new interests** that don't exist yet
3. **Soft-delete interests** not in the new list
4. **Generate embeddings** for new interests using configured LLM model
5. **Update metadata** for interest verification

### Important Constraints
- Only the **most recent request** per user will be executed
- Older requests within the 500ms window are cancelled
- Request ID is NOT correlated with database transaction ID
- Empty array `[]` results in ALL interests being soft-deleted

## Use Cases

### Initial Setup
```json
{
  "interests": ["machine learning", "AI", "deep learning"]
}
```
First-time user setting up research interests.

### Updating Interests
```json
{
  "interests": ["machine learning", "NLP", "computer vision"]
}
```
Adds "NLP" and "computer vision", removes or keeps others based on previous state.

### Restoring Deleted Interest
```json
{
  "interests": ["machine learning", "NLP"]
}
```
If "NLP" was previously deleted, it will be restored automatically.

### Clearing All Interests
```json
{
  "interests": []
}
```
Removes all interests (soft-delete).

### Typing in UI (High-Frequency Scenario)
User types: "machine lear" → "machine learning" → "machine learning,"
System handles multiple rapid requests efficiently, only applies final state.

## Important Notes & Warnings

### Asynchronous Nature
⚠️ **This is an asynchronous operation**:
- Returns immediately with request ID
- Database update happens ~500ms later
- Results not immediately available
- Use eventual consistency expectations

### Latest Request Only
⚠️ **Only the most recent request is executed**:
- If user sends 10 requests in rapid succession, only the last one matters
- Previous 9 requests are automatically discarded
- This is intentional behavior to optimize for typing scenarios

### Interest Embeddings
- Each interest generates an embedding using configured LLM model
- Embeddings enable semantic similarity matching during paper verification
- Embeddings generation is also part of the delayed async processing

### Verification Impact
- Changes take effect after the 500ms delay
- New verifications will use updated interests
- In-progress verifications are not affected
- Trigger new verification via `POST /verify` to see updated results

### Edge Cases
- **Empty array**: All interests soft-deleted (not permanently removed)
- **Exceeds max limit**: Request rejected with 400 error before queuing
- **Duplicate interests**: Handled gracefully by database constraints
- **Special characters**: Supported, but may affect embedding quality
- **Very long interests**: May be truncated by LLM model limits
- **Invalid requests**: Validated before queuing

## Error Handling
- **400 Error**: Invalid request format, validation failure, or exceeds maximum interests limit
- **401 Error**: Unauthorized - no valid authentication
- **500 Error**: Failed to queue update request (Redis/queue issues)

## Best Practices
1. **Wait after submission**: Don't immediately query interests (wait >500ms)
2. **Single final submission**: Send one update with complete final list
3. **Respect the limit**: Maximum interests count is limited by `rss.max_prompt_number` (check system configuration)
4. **Clear descriptions**: Use specific, clear interest keywords for better matching
5. **Trigger verification**: After updating interests, call `POST /verify` to re-verify papers

## Performance Characteristics
- **Latency**: ~500ms delay before database operations
- **Throughput**: Optimized for high-frequency requests (typing scenarios)
- **Scaling**: Uses Redis queuing for distributed systems
- **Token Usage**: Each new interest consumes LLM API tokens for embedding generation

## Related Endpoints
- **`GET /interests`**: Retrieve current active interests
- **`POST /verify`**: Trigger paper verification with updated interests
- **`POST /stream-verify`**: Stream verification progress with live updates
- **`GET /all-verified-papers`**: View papers matched to your interests
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

    // Validate max interests limit
    let max_count = state.config.rss.max_prompt_number;
    if payload.interests.len() > max_count {
        return Err(ApiError::CustomError {
            message: format!(
                "Exceeded maximum interests limit: {} (provided: {})",
                max_count,
                payload.interests.len()
            ),
            code: ApiCode::COMMON_FEED_ERROR,
        });
    }

    let config = app_config();

    // Create UpdateTaskManager
    let manager = UpdateTaskManager::new(
        state.redis.pool.clone(),
        state.config.rss.feed_redis.redis_prefix.clone(),
        state.config.rss.feed_redis.redis_key_default_expire,
    );

    let request_id = manager
        .submit_update(
            UpdateTaskInput {
                task_type: TaskType::UserInterests,
                user_id: user.id,
                data: UpdateTaskData::UserInterests {
                    interests: payload.interests,
                    version: config.llm.model.clone(),
                },
                request_id: Uuid::new_v4().to_string(),
            },
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

    // Return request_id immediately (do not wait for database operation)
    Ok(ApiResponse::data(request_id))
}
