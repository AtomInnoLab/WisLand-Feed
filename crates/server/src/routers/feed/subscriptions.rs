use axum::Json;
use axum::extract::{Path, State};
use common::{error::api_error::*, prelude::ApiCode};
use feed::redis::update_task_manager::{
    TaskType, UpdateTaskData, UpdateTaskInput, UpdateTaskManager,
};
use seaorm_db::{
    entities::feed::rss_subscriptions, query::feed::rss_subscriptions::RssSubscriptionsQuery,
};
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
    path = "/subscriptions",
    summary = "Get user's RSS subscriptions",
    description = r#"
Retrieve all RSS subscriptions for the authenticated user.

## Overview
This endpoint returns a list of all subscription records for the current user, showing which RSS sources they are subscribed to.

## Returns
Returns an array of `rss_subscriptions::Model` objects, each containing:
- `id`: Subscription record ID (unique identifier for the subscription)
- `user_id`: User ID who owns this subscription
- `source_id`: RSS source ID being subscribed to
- `created_at`: Timestamp when the subscription was created
- `updated_at`: Timestamp of last update

## Use Cases
- Display user's subscription list
- Manage subscriptions (before modifying)
- Show subscribed feeds in UI
- Sync subscription status

## Related Endpoints
- Use `POST /subscriptions` to batch update subscriptions
- Use `POST /subscriptions/one` to add a single subscription
- Use `DELETE /subscriptions/{id}` to remove a subscription
- Use `GET /user_rss` to get RSS source details for subscribed feeds
"#,
    responses(
        (status = 200, body = Vec<rss_subscriptions::Model>, description = "Successfully retrieved user's subscriptions"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error"),
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
    summary = "Batch update RSS subscriptions",
    description = r#"
Batch update user's RSS subscriptions with a new set of source IDs using incremental update logic and advanced asynchronous processing with optimization for high-frequency updates.

## Overview
This endpoint allows users to batch update their RSS subscriptions. The system uses these subscriptions to control which RSS sources deliver papers to the user. The update is processed asynchronously with a delayed execution mechanism using incremental update logic.

## Request Body
```json
{
  "source_ids": [1, 2, 3, 4, 5]
}
```

### Parameters
- `source_ids` (required): Array of RSS source IDs to subscribe to. Empty array `[]` is allowed and will clear all subscriptions.

## Behavior & Update Logic

### Asynchronous Processing with 500ms Delay
This endpoint uses a sophisticated delayed execution mechanism:
- **Request Queuing**: Requests are queued for processing
- **500ms Delay**: There's a 500ms delay before actual database operations
- **Latest-Wins Strategy**: Only the most recent request per user is executed
- **Auto-Cancel**: Older requests are automatically cancelled if a new request arrives within the delay period

### Incremental Update Strategy
The update performs set-based incremental operations (not a complete replacement):
- **Intersection (A∩B)**: Existing subscriptions that appear in the new request are preserved. If they were previously soft-deleted, they are restored (soft-delete cleared).
- **A-B (New subscriptions)**: Source IDs in the request that don't exist are created as new subscription records.
- **B-A (Removed subscriptions)**: Existing subscriptions not in the new request are soft-deleted (not permanently removed).
- **Duplicate source IDs** are automatically deduplicated
- **Empty array** will soft-delete all subscriptions (unsubscribe from all sources)

### High-Frequency Optimization
The system is optimized for rapid, repeated updates (e.g., user selecting multiple feeds):
- Multiple requests within 500ms are automatically merged
- Only the final state is applied to the database
- Reduces database load and prevents race conditions
- Makes the UI responsive to user interactions

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
1. **Restore soft-deleted subscriptions** that match the request (intersection)
2. **Create new subscriptions** for source IDs not already subscribed to (A-B)
3. **Soft-delete subscriptions** not in the new request (B-A)
4. **Deduplicate source IDs** automatically
5. **Empty array handling**: If `source_ids` is empty, all subscriptions are soft-deleted

### Important Constraints
- Only the **most recent request** per user will be executed
- Older requests within the 500ms window are cancelled
- Request ID is NOT correlated with database transaction ID
- Empty array `[]` results in ALL subscriptions being soft-deleted (can be restored)
- Removed subscriptions are **soft-deleted**, not permanently deleted

## Use Cases

### Initial Setup
```json
{
  "source_ids": [1, 5, 12, 23]
}
```
First-time user setting up RSS subscriptions.

### Updating Subscriptions
```json
{
  "source_ids": [1, 5, 15, 20, 25]
}
```
Replace existing subscriptions with a new set of sources.

### Clearing All Subscriptions
```json
{
  "source_ids": []
}
```
Remove all subscriptions (unsubscribe from all sources).

### Rapid Selection (High-Frequency Scenario)
User quickly selects/deselects multiple feeds in UI:
- System handles multiple rapid requests efficiently
- Only applies final state after user stops interacting

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
- This is intentional behavior to optimize for UI interactions

### Incremental Update
⚠️ **This is an incremental update operation**:
- Uses set-based operations (intersection, create, soft-delete)
- Preserves existing subscriptions that match the request
- Only creates new and removes missing subscriptions
- Removed subscriptions are soft-deleted (can be restored)
- Use `POST /subscriptions/one` to add a single subscription without affecting others

### Edge Cases
- **Empty array**: All subscriptions soft-deleted (can be restored)
- **Duplicate source IDs**: Automatically deduplicated
- **Invalid source IDs**: May cause operation to fail at database level
- **Very large arrays**: Performance may degrade with extremely large subscription lists

## Error Handling
- **400 Error**: Invalid request format or validation failure
- **401 Error**: Unauthorized - no valid authentication
- **500 Error**: Failed to queue update request (Redis/queue issues)

## Best Practices
1. **Wait after submission**: Don't immediately query subscriptions (wait >500ms)
2. **Single final submission**: Send one update with complete final list
3. **Reasonable subscription count**: Keep number of subscriptions manageable
4. **Check source validity**: Ensure source IDs exist before subscribing
5. **Use single endpoint**: For adding one subscription, prefer `POST /subscriptions/one`

## Performance Characteristics
- **Latency**: ~500ms delay before database operations
- **Throughput**: Optimized for high-frequency requests (UI interaction scenarios)
- **Scaling**: Uses Redis queuing for distributed systems

## Related Endpoints
- **`GET /subscriptions`**: Retrieve current active subscriptions
- **`POST /subscriptions/one`**: Add a single subscription without affecting others
- **`DELETE /subscriptions/{id}`**: Remove a specific subscription
- **`GET /rss`**: Browse available RSS sources
"#,
    request_body = SubscriptionsCreateRequest,
    responses(
        (status = 200, description = "Successfully queued subscriptions update, returns request ID for tracking", body = String),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 400, description = "Invalid request data"),
        (status = 500, description = "Failed to queue update request"),
    ),
    tag = FEED_TAG,
)]
pub async fn batch_subscriptions(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<SubscriptionsCreateRequest>,
) -> Result<ApiResponse<String>, ApiError> {
    let count = payload.source_ids.len();
    tracing::info!(user_id = user.id, count, "set subscriptions (async)");
    if count == 0 {
        tracing::info!(
            user_id = user.id,
            "empty source_ids: clear all subscriptions"
        );
    }

    // Create UpdateTaskManager
    let manager = UpdateTaskManager::new(
        state.redis.pool.clone(),
        state.config.rss.feed_redis.redis_prefix.clone(),
        state.config.rss.feed_redis.redis_key_default_expire,
        state.conn.clone(),
        state.redis.pubsub_manager.clone(),
        state.config.rss.verify_papers_channel.clone(),
        state.config.rss.update_task_merge_delay_ms.unwrap_or(500),
    );

    let request_id = manager
        .submit_update(
            UpdateTaskInput {
                task_type: TaskType::UserSubscriptions,
                user_id: user.id,
                data: UpdateTaskData::UserSubscriptions {
                    source_ids: payload.source_ids,
                },
                request_id: Uuid::new_v4().to_string(),
            },
            state.redis.apalis_conn.clone(),
        )
        .await
        .map_err(|e| ApiError::CustomError {
            message: format!("Failed to submit subscriptions update: {e}"),
            code: ApiCode::COMMON_FEED_ERROR,
        })?;

    tracing::info!(
        user_id = user.id,
        request_id = %request_id,
        "Successfully queued subscriptions update"
    );

    // Return request_id immediately (do not wait for database operation)
    Ok(ApiResponse::data(request_id))
}

#[utoipa::path(
    post,
    path = "/subscriptions/one",
    summary = "Add a single RSS subscription",
    description = r#"
Add a single RSS source subscription for the authenticated user.

## Overview
This endpoint adds one RSS source subscription to the user's existing subscriptions without affecting other subscriptions.

## Request Body
```json
{
  "source_id": 42
}
```

## Parameters
- `source_id`: The RSS source ID to subscribe to

## Behavior
- **Append Operation**: Does NOT remove existing subscriptions
- Only adds the specified source to the user's subscription list
- Idempotent: If already subscribed, returns `null` (no error)
- If the source doesn't exist, returns `null` (no error)

## Returns
Returns an `Option<i64>`:
- `Some(id)`: Subscription was created successfully, returns the new subscription ID
- `null`: Subscription already exists OR source doesn't exist (no action taken)

## Response Examples

**Success - New subscription created:**
```json
123
```

**Already exists or invalid source:**
```json
null
```

## Use Cases
- Add a single feed subscription
- Subscribe to a new RSS source
- Incremental subscription management
- One-click subscribe functionality

## Comparison with Batch Endpoint
| Feature | `/subscriptions` (batch) | `/subscriptions/one` (single) |
|---------|-------------------------|------------------------------|
| Operation | Replace all | Append one |
| Existing subscriptions | Removed | Preserved |
| If already subscribed | Creates anyway | Returns null |
| Multiple sources | Yes | No |

## Related Endpoints
- Use `POST /subscriptions` for batch subscription replacement
- Use `DELETE /subscriptions/{id}` to remove a subscription
- Use `GET /subscriptions` to view all current subscriptions
"#,
    request_body = SubscriptionCreateOneRequest,
    responses(
        (status = 200, description = "Returns subscription ID if created, or null if already exists or source invalid", body = Option<i64>),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error"),
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
    summary = "Delete a single RSS subscription",
    description = r#"
Remove a single RSS subscription for the authenticated user.

## Overview
This endpoint deletes a specific subscription record by its ID, unsubscribing the user from that RSS source.

## Parameters
- `subscription_id`: The unique ID of the subscription record to delete (from `GET /subscriptions`)

## Important Note
The parameter is the **subscription record ID**, NOT the RSS source ID.
- Subscription ID: Unique identifier for the user-source relationship
- Source ID: The RSS feed's identifier

To get subscription IDs, first call `GET /subscriptions`.

## Returns
Returns `true` if the deletion was successful.

## Behavior
- Removes the specified subscription record
- User will no longer receive papers from this source
- Does not affect other users' subscriptions to the same source
- Does not delete the RSS source itself

## Use Cases
- Unsubscribe from a single RSS feed
- Manage subscription list
- Remove unwanted feeds
- Clean up subscriptions

## Error Handling
- If subscription ID doesn't exist: Operation may succeed silently
- If subscription belongs to another user: Typical database constraints apply

## Example Workflow
1. Call `GET /subscriptions` to get subscription list
2. Find the subscription record you want to delete
3. Use its `id` field (not `source_id`) in this endpoint
4. Subscription is removed

## Related Endpoints
- Use `GET /subscriptions` to get subscription IDs
- Use `POST /subscriptions` to batch replace all subscriptions
- Use `POST /subscriptions/one` to add a subscription
"#,
    params(
        ("subscription_id" = i64, Path, description = "The unique identifier of the subscription record (not the source ID) to delete"),
    ),
    responses(
        (status = 200, description = "Subscription deleted successfully, returns true", body = bool),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 404, description = "Subscription not found"),
        (status = 500, description = "Database error"),
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
