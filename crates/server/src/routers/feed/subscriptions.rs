use axum::Json;
use axum::extract::{Path, State};
use common::{error::api_error::*, prelude::ApiCode};
use feed::redis::verify_manager::VerifyManager;
use seaorm_db::{
    entities::feed::rss_subscriptions, query::feed::rss_subscriptions::RssSubscriptionsQuery,
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
Replace all user's RSS subscriptions with a new set of source IDs.

## Overview
This endpoint performs a complete replacement of the user's RSS subscriptions. All existing subscriptions are removed and replaced with the new list.

## Request Body
```json
{
  "source_ids": [1, 2, 3, 4, 5]
}
```

## Parameters
- `source_ids`: Array of RSS source IDs to subscribe to

## Behavior
- **Replace Operation**: This is NOT an append operation
- All existing subscriptions for the user are deleted first
- New subscriptions are created for the provided source IDs
- Duplicate source IDs are automatically deduplicated
- Empty array `[]` will **unsubscribe from all sources**

## Returns
Returns an array of subscription IDs (i64) for the newly created or existing subscription records.

## Use Cases
- Bulk subscription management
- Import/export subscription lists
- Subscription synchronization
- Reset subscriptions to a new set

## Important Notes
- **This is a complete replacement operation**
- Providing an empty array `[]` will clear all subscriptions
- Invalid source IDs may cause the operation to fail
- Operation is atomic - either all succeed or all fail

## Special Behavior
When `source_ids` is empty:
- Logs: "empty source_ids: clear all subscriptions"
- All user's subscriptions will be removed
- Returns an empty array

## Related Endpoints
- Use `GET /subscriptions` to view current subscriptions before updating
- Use `POST /subscriptions/one` to add a single subscription without affecting others
- Use `GET /rss` to browse available RSS sources
"#,
    request_body = SubscriptionsCreateRequest,
    responses(
        (status = 200, description = "Successfully updated subscriptions, returns list of subscription IDs", body = Vec<i64>),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 400, description = "Invalid source IDs"),
        (status = 500, description = "Database error or transaction failed"),
    ),
    tag = FEED_TAG,
)]
pub async fn batch_subscriptions(
    State(state): State<AppState>,
    User(user): User,
    Json(payload): Json<SubscriptionsCreateRequest>,
) -> Result<ApiResponse<Vec<i64>>, ApiError> {
    let count = payload.source_ids.len();
    tracing::info!(user_id = user.id, count, "create subscriptions request");
    if count == 0 {
        tracing::info!(
            user_id = user.id,
            "empty source_ids: clear all subscriptions"
        );
    }

    let verify_manager = VerifyManager::new(
        state.redis.clone().pool,
        state.conn.clone(),
        state.config.rss.feed_redis.redis_prefix.clone(),
        state.config.rss.feed_redis.redis_key_default_expire,
    )
    .await;

    verify_manager.finish_user_verify(user.id, true).await?;

    // Update subscriptions with current paper IDs
    RssSubscriptionsQuery::update_subscription_latest_paper_ids(&state.conn, user.id, None)
        .await
        .context(DbErrSnafu {
            stage: "update-subscription-latest-paper-ids",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    let ids = RssSubscriptionsQuery::replace_many(&state.conn, user.id, payload.source_ids)
        .await
        .context(DbErrSnafu {
            stage: "create-rss-subscriptions",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    tracing::info!(
        user_id = user.id,
        returned = ids.len(),
        "create subscriptions done"
    );
    Ok(ApiResponse::data(ids))
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
