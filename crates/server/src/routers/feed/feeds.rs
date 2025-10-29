use super::FEED_TAG;
use crate::model::page::{Page, Pagination, de_opt_i32_from_any};
use crate::{
    middlewares::auth::{User, UserInfo},
    model::base::ApiResponse,
    state::app_state::AppState,
};
use axum::Json;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use chrono::{DateTime, FixedOffset};
use common::{error::api_error::*, prelude::ApiCode};
use feed::dispatch;
use feed::services::{ConnectionMonitor, SseMessageHandler, VerifyService, create_verify_stream};
use feed::workers::verify_user_papers::VerifyAllUserPapersInput;
use futures::stream::Stream;
use seaorm_db::query::feed::user_paper_verifications::{
    ListVerifiedParams, MarkReadParams, PaperWithVerifications, UserPaperVerificationsQuery,
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
    /// Whether to ignore pagination and return all data (optional, defaults to false)
    pub ignore_pagination: Option<bool>,
    pub channel: Option<String>,
    pub matches: Option<String>,
    pub user_interest_ids: Option<String>,
    // #[serde(flatten)]
    // pub time_range: Option<TimeRangeParam>,
    // pub ignore_time_range: Option<bool>,
    pub keyword: Option<String>,
    #[serde(default, deserialize_with = "de_opt_i32_from_any")]
    pub rss_source_id: Option<i32>,
}

/// OpenAPI params declaration: avoid type degradation to string caused by combination of `#[serde(flatten)]` and `IntoParams`
#[derive(Debug, utoipa::IntoParams)]
pub struct AllVerifiedPapersParams {
    /// Page number (starts from 1)
    pub page: Option<i32>,
    /// Number of items per page
    pub page_size: Option<i32>,
    /// Whether to ignore pagination and return all data
    pub ignore_pagination: Option<bool>,
    pub channel: Option<String>,
    /// Comma-separated match types: yes,no,partial
    pub matches: Option<String>,
    /// Comma-separated interest IDs
    pub user_interest_ids: Option<String>,
    /// Start datetime for filtering papers
    pub start: Option<DateTime<FixedOffset>>,
    /// End datetime for filtering papers
    pub end: Option<DateTime<FixedOffset>>,
    /// Ignore time range filter
    pub ignore_time_range: Option<bool>,
    /// Search keyword to filter papers by title or content
    pub keyword: Option<String>,
    /// Filter papers by specific RSS source ID
    pub rss_source_id: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct AllVerifiedPapersResponse {
    pub pagination: Pagination,
    pub papers: Vec<PaperWithVerifications>,
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
    pub max_match_limit_per_user: Option<i32>,
    pub search_params: Option<ListVerifiedParams>,
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
Initiate the asynchronous verification process for user's papers against their interests.

## Overview
This endpoint triggers an asynchronous verification job that matches unverified papers from the user's RSS subscriptions against their defined interests. The verification process uses AI to determine relevance by comparing paper content with user's interest keywords.

## Request Body

```json
{
  "channel": "default"
}
```

### Parameters
- `channel` (required): The channel to filter papers for verification. Only papers from RSS sources in this channel will be considered.

## Process Flow
1. **Immediate Response**: Returns `true` immediately upon successful job queuing
2. **Background Processing**: Verification runs asynchronously via worker processes
3. **AI Matching**: Each unverified paper is evaluated against all user interests using semantic similarity
4. **Result Classification**: Papers are classified as "Yes" (relevant), "No" (not relevant), or "Partial" (somewhat relevant)
5. **Status Updates**: Real-time progress available via `/stream-verify` SSE endpoint

## Job Configuration
The verification job uses system-configured limits:
- `max_prompt_number`: Maximum number of prompts per batch
- `max_rss_paper`: Maximum number of RSS papers to process per user

## Returns
Returns `true` (wrapped in `ApiResponse<bool>`) if the verification job was successfully queued.

**Response Structure:**
```json
{
  "success": true,
  "message": "Success",
  "data": true
}
```

## Asynchronous Behavior
⚠️ **Important**: This is an asynchronous operation
- Returns immediately after queuing the job
- Actual verification happens in background worker processes
- No progress is returned in the response
- Use other endpoints to track progress and results

## Progress Tracking
After triggering verification, use these endpoints to track progress:

1. **`POST /stream-verify`**: Real-time SSE stream with live updates
   - Shows progress as papers are verified
   - Provides verified paper details in real-time
   - Best for showing live progress in UI

2. **`GET /all-verified-papers`**: Retrieve verified papers
   - Fetch all verified papers after completion
   - Use after verification finishes

3. **`GET /all-users-verify-info`**: Get verification statistics
   - Shows pending, success, fail counts
   - Useful for progress monitoring

## Verification Logic
- **Input**: Unverified papers from user's RSS subscriptions in the specified channel
- **Processing**: Each paper is compared against ALL user interests using AI
- **Output**: Verification records linking papers to interests with match scores
- **Classification**: Each verification marked as "Yes", "No", or "Partial"

## Error Scenarios
- **500 Error**: Failed to queue verification job (Redis connection issue, queue full)
- **401 Error**: Unauthorized - no valid authentication token
- **Invalid channel**: Job may queue but process no papers if channel doesn't exist

## Use Cases
- Trigger verification after adding new RSS subscriptions
- Re-verify papers after updating interests
- Verify papers from a specific channel
- Initial verification for new users
- Batch process unverified papers

## Important Notes
- Multiple calls will create multiple jobs (they are additive, not replaced)
- Verification can be time-consuming for users with many papers
- Token usage counts toward API rate limits
- Only processes papers from subscribed RSS sources
- Uses the latest user interests for verification

## Related Endpoints
- **`POST /stream-verify`**: Stream verification progress in real-time
- **`GET /all-verified-papers`**: Retrieve verified papers
- **`GET /all-users-verify-info`**: Check verification statistics
- **`GET /unverified-count-info`**: See how many papers await verification
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
    path = "/all-verified-papers",
    summary = "Get all verified papers",
    description = r#"
Retrieve a paginated or complete list of all verified papers for the authenticated user.

## Overview
This endpoint returns papers that have been verified against the user's interests, with various filtering and pagination options. The response includes comprehensive metadata including paper details, verification results, interest mappings, and source information.

## Query Parameters

### Pagination Parameters
- `page` (optional, default: 1): Page number for pagination. Starts from 1. Invalid or non-positive values default to 1.
- `page_size` (optional, default: 20): Number of items per page. Invalid or non-positive values default to 20.
- `ignore_pagination` (optional, default: false): When `true`, returns all data without pagination. When `false`, uses pagination with default values.

**Pagination Behavior:**
- If both `page` and `page_size` are not provided, defaults to `page=1, page_size=20`
- If either parameter is invalid (non-positive), uses the default value
- When `ignore_pagination=true`, returns ALL data and `pagination` info reflects the total dataset

### Filtering Parameters
- `channel` (optional): Filter by specific channel name (e.g., "arxiv", "default"). Only returns papers from matching channel.
- `user_interest_ids` (optional): Filter by specific interest IDs as comma-separated string (e.g., "1,2,3,4").
  - Empty string or spaces are ignored (same as not providing the parameter)
  - Only returns papers that match at least one of the specified interests
- `keyword` (optional): Search keyword to filter papers by title or content. Performs substring matching.
- `rss_source_id` (optional): Filter papers by specific RSS source ID. Only shows papers from that exact source.

### Deprecated/Not Implemented Parameters
⚠️ **Note:** The following parameters are declared but not currently implemented:
- `matches` (optional): Declared but parsing logic is commented out. Passing values will have no effect.
- `start` (optional): Time range start. Declared but not implemented.
- `end` (optional): Time range end. Declared but not implemented.
- `ignore_time_range` (optional): Declared but not implemented.

## Returns
Returns an `AllVerifiedPapersResponse` object containing:

### Pagination Object
- `page` (i32): Current page number
- `page_size` (i32): Items per page
- `total` (u64): Total number of papers matching the filter criteria
- `total_pages` (u64): Total number of pages

When `ignore_pagination=true`:
- `page`: Set to 1
- `page_size`: Set to total count
- `total_pages`: Set to 1

### Papers Array
Array of `PaperWithVerifications` objects, each containing:
- Paper metadata: id, title, link, description, author, pub_date, etc.
- Verification results for each matching interest
- Status indicators and metadata

### Interest Map
- `HashMap<i64, String>`: Mapping of interest IDs to interest names
- Keys are user interest IDs
- Values are the interest keywords/phrases

### Source Map
- `HashMap<i32, rss_sources::Model>`: Mapping of RSS source IDs to complete source details
- Keys are source IDs
- Values include: id, channel, name, url, description, logo_img, background_img, timestamps

## Example Requests

### Paginated Request (Default)
```
GET /all-verified-papers?page=1&page_size=20
```
Returns first 20 papers.

### Get All Data (No Pagination)
```
GET /all-verified-papers?ignore_pagination=true
```
Returns ALL verified papers for the user, regardless of count.

### Filter by Channel
```
GET /all-verified-papers?channel=arxiv&page=1&page_size=10
```
Returns first 10 papers from the "arxiv" channel.

### Filter by Interests
```
GET /all-verified-papers?user_interest_ids=1,2,3
```
Returns all papers that match interests with IDs 1, 2, or 3.

### Search by Keyword
```
GET /all-verified-papers?keyword=machine%20learning
```
Returns papers whose title or content contains "machine learning".

### Filter by Source
```
GET /all-verified-papers?rss_source_id=42
```
Returns all papers from RSS source with ID 42.

### Combined Filters
```
GET /all-verified-papers?channel=arxiv&keyword=neural&user_interest_ids=1,2&page=2&page_size=50
```
Returns page 2 (items 51-100) of arxiv papers containing "neural" and matching interests 1 or 2.

## Example Response

```json
{
  "success": true,
  "message": "Success",
  "data": {
    "pagination": {
      "page": 1,
      "page_size": 20,
      "total": 156,
      "total_pages": 8
    },
    "papers": [
      {
        "id": 789,
        "title": "Example Paper Title",
        "link": "https://example.com/paper",
        "description": "Paper description...",
        "author": "John Doe",
        "pub_date": "2024-01-01T00:00:00Z",
        "channel": "arxiv",
        "verifications": [
          {
            "id": 123,
            "match": "Yes",
            "relevance_score": 0.95,
            "interest_id": 1
          }
        ]
      }
    ],
    "interest_map": {
      "1": "Machine Learning",
      "2": "Natural Language Processing"
    },
    "source_map": {
      "42": {
        "id": 42,
        "channel": "arxiv",
        "name": "AI Research",
        "url": "https://arxiv.org/feed",
        "description": "Latest AI research papers",
        "logo_img": null,
        "background_img": null,
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "last_fetched_at": "2024-01-01T10:00:00Z"
      }
    }
  }
}
```

## Use Cases
- Display verified papers in feed UI with pagination
- Export all verified papers (using `ignore_pagination=true`)
- Filter by specific topics of interest
- Search for papers by keyword
- Show papers from specific RSS sources
- Browse verified papers by channel

## Related Endpoints
- Use `POST /verify` to trigger verification of unverified papers
- Use `GET /unverified-papers` to see papers awaiting verification
- Use `POST /mark-as-read` to mark papers as read
- Use `POST /batch-delete` to delete multiple papers
"#,
    params(
        AllVerifiedPapersParams
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
    tracing::info!("user: {:?}, payload: {:?}", user, payload);

    // let verify_service = VerifyService::new(
    //     state.redis.clone().pool,
    //     state.conn.clone(),
    //     state.config.rss.feed_redis.redis_prefix.clone(),
    //     state.config.rss.feed_redis.redis_key_default_expire,
    // )
    // .await;

    // // Parse comma-separated matches string to Vec<VerificationMatch>
    // let parsed_matches: Option<Vec<VerificationMatch>> =
    //     payload.matches.as_ref().and_then(|matches_str| {
    //         if matches_str.trim().is_empty() {
    //             None
    //         } else {
    //             let matches: Result<Vec<VerificationMatch>, _> = matches_str
    //                 .split(',')
    //                 .map(|s| s.trim())
    //                 .filter(|s| !s.is_empty())
    //                 .map(|s| match s.to_lowercase().as_str() {
    //                     "yes" => Ok(VerificationMatch::Yes),
    //                     "no" => Ok(VerificationMatch::No),
    //                     "partial" => Ok(VerificationMatch::Partial),
    //                     _ => Err(format!("Invalid match value: {s}")),
    //                 })
    //                 .collect();
    //             matches.ok()
    //         }
    //     });

    // Parse comma-separated user_interest_ids string to Vec<i64>
    let parsed_user_interest_ids: Option<Vec<i64>> =
        payload.user_interest_ids.as_ref().and_then(|ids_str| {
            if ids_str.trim().is_empty() {
                None
            } else {
                let ids: Result<Vec<i64>, _> = ids_str
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse::<i64>())
                    .collect();
                ids.ok()
            }
        });

    // Check if pagination should be ignored
    let use_pagination = !payload.ignore_pagination.unwrap_or(false);

    // If pagination is enabled, use pagination; otherwise return all data
    let (offset, limit) = if use_pagination {
        (
            Some(payload.pagination.offset()),
            Some(payload.pagination.page_size()),
        )
    } else {
        (None, None)
    };

    let verified_papers = UserPaperVerificationsQuery::list_verified_by_user(
        &state.conn,
        user.id,
        ListVerifiedParams {
            channel: payload.channel.clone(),
            user_interest_ids: parsed_user_interest_ids,
            offset, // Use calculated offset
            limit,  // Use calculated limit
            keyword: payload.keyword.clone(),
            rss_source_id: payload.rss_source_id,
            ignore_pagination: payload.ignore_pagination,
        },
    )
    .await
    .context(DbErrSnafu {
        stage: "list-verified-papers",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;

    // Query user interests and subscription sources in parallel
    let (interest_items_result, subscriptions_result) = tokio::join!(
        UserInterestsQuery::list_by_user_id(&state.conn, user.id),
        RssSubscriptionsQuery::list_by_user_id(&state.conn, user.id, None)
    );

    let interest_items = interest_items_result.context(DbErrSnafu {
        stage: "list-user-interests",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;
    let interest_map: HashMap<i64, String> = interest_items
        .into_iter()
        .map(|m| (m.id, m.interest))
        .collect();

    let subscriptions = subscriptions_result.context(DbErrSnafu {
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
        pagination: if use_pagination {
            Pagination {
                page: payload.pagination.page(),
                page_size: payload.pagination.page_size(),
                total: verified_papers.total,
                total_pages: verified_papers.total / payload.pagination.page_size() as u64,
            }
        } else {
            // When not using pagination, return pagination info for all data
            Pagination {
                page: 1,
                page_size: verified_papers.total as i32,
                total: verified_papers.total,
                total_pages: 1,
            }
        },
        papers: verified_papers.items,
        interest_map,
        source_map,
    }))
}

#[utoipa::path(
    post,
    path = "/mark-as-read",
    summary = "Mark papers as read",
    description = r#"
Mark one or more verified papers as read for the authenticated user.

## Overview
This endpoint allows users to mark verified papers as read, updating their `unread` status in the database. This operation is used to track user's reading progress and filter unread papers.

## Request Body

```json
{
  "paper_ids": [1, 2, 3, 4, 5],
  "channel": "arxiv",
  "read_all": false
}
```

### Parameters
- `paper_ids` (required): Array of paper IDs to mark as read. Should be IDs of verified papers for the authenticated user.
- `channel` (optional): Channel filter. When provided, only papers from this channel will be affected. If not provided, no channel filtering is applied.
- `read_all` (required, boolean): When `true`, marks ALL user's papers as read (ignores `paper_ids`). When `false`, marks only the specified `paper_ids`.

## Behavior Modes

### Mode 1: Mark Specific Papers (`read_all=false`)
```json
{
  "paper_ids": [1, 2, 3],
  "read_all": false
}
```
- Marks only the specified paper IDs as read
- If any ID doesn't exist or doesn't belong to the user, it's silently ignored
- Returns count of actually marked papers (may be less than provided IDs)

### Mode 2: Mark All Papers (`read_all=true`)
```json
{
  "paper_ids": [],  // Ignored when read_all=true
  "read_all": true
}
```
- Marks ALL user's verified papers as read
- `paper_ids` array is ignored
- Useful for "mark all as read" functionality
- More efficient for bulk operations

### Mode 3: Mark All in Channel
```json
{
  "paper_ids": [],  // Ignored when read_all=true
  "channel": "arxiv",
  "read_all": true
}
```
- Marks all papers from the specified channel as read
- Limits scope to a specific channel
- Useful for channel-specific "mark all as read"

## Returns
Returns a `u64` representing the number of papers successfully marked as read.

Examples:
- `0`: No papers were marked (e.g., invalid IDs)
- `5`: 5 papers were marked as read
- `156`: All 156 papers were marked (when using `read_all=true`)

## Important Notes
- This operation only affects verified papers (not unverified)
- Non-existent or invalid paper IDs are silently ignored (not counted in return value)
- The operation is performed in a single transaction (all or nothing)
- Marking papers as read removes them from "unread" counts and filter lists
- Can be called multiple times safely (idempotent operation)
- `read_all=true` overrides the `paper_ids` parameter

## Error Handling
- Invalid or missing `read_all` flag: Request rejected with validation error
- Empty `paper_ids` with `read_all=false`: Returns 0 (no papers marked)

## Example Requests

### Mark Individual Papers
**Request:**
```json
{
  "paper_ids": [42, 1337],
  "read_all": false
}
```
**Response:**
```json
{
  "success": true,
  "message": "Success",
  "data": 2
}
```
Returns 2 if both papers were successfully marked.

### Mark All Papers
**Request:**
```json
{
  "paper_ids": [],
  "read_all": true
}
```
**Response:**
```json
{
  "success": true,
  "message": "Success",
  "data": 156
}
```
Returns total count of all verified papers marked as read.

### Mark All in Specific Channel
**Request:**
```json
{
  "paper_ids": [],
  "channel": "arxiv",
  "read_all": true
}
```
**Response:**
```json
{
  "success": true,
  "message": "Success",
  "data": 42
}
```
Returns count of papers from "arxiv" channel marked as read.

## Use Cases
- User reads a paper and marks it as read
- Batch mark multiple papers as read after review
- "Mark all as read" for all verified papers
- "Mark channel as read" for specific RSS channel
- Reset unread counts
- Clean up read status when archiving papers

## Related Endpoints
- Use `GET /all-verified-papers` to retrieve papers (filter by unread status)
- Use `GET /unread-count` to get count of unread papers
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
  "max_match_limit_per_user": 50,
  "search_params": null
}
```

### Parameters
- `channel` (optional): Channel to filter papers for verification. When provided, only papers from this channel will be verified.
- `max_match_limit_per_user` (optional): Maximum number of matched papers per user. Defaults to system configuration value (`max_match_limit_per_user`). When the matched paper count reaches this limit:
  - A `match_limit_reached` event is sent
  - The SSE connection is automatically closed
  - Further processing stops to prevent exceeding limits
- `search_params` (optional): Advanced filtering parameters for papers to include in verification. When `null` or not provided, all unverified papers are included. Structure:
  ```json
  {
    "user_interest_ids": [1, 2, 3],
    "keyword": "machine learning",
    "rss_source_id": 42,
    "offset": null,
    "limit": null,
    "channel": "arxiv",
    "ignore_pagination": true
  }
  ```

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

    // Create connection monitor, automatically triggers Drop when SSE stream ends
    let monitor = ConnectionMonitor::new(
        user_id,
        state.redis.pubsub_manager.clone(),
        verify_papers_sub_channel.clone(),
    );

    // Create broadcast channel for Redis PubSub message forwarding
    let (tx, rx) = broadcast::channel::<String>(1000);

    // Create message handler to forward Redis messages to SSE stream
    let handler = Box::new(SseMessageHandler::new(
        user_id,
        verify_papers_sub_channel,
        tx,
    ));

    // Start listener in separate task to avoid blocking
    let mut pubsub_manager = state.redis.pubsub_manager.clone();
    tokio::spawn(async move {
        pubsub_manager.add_listener(handler).await;
    });

    let verify_service = VerifyService::new(
        state.redis.clone().pool,
        state.conn.clone(),
        state.redis.pubsub_manager.clone(),
        state.config.rss.feed_redis.redis_prefix.clone(),
        state.config.rss.feed_redis.redis_key_default_expire,
        state.config.rss.verify_papers_channel.clone(),
    )
    .await;

    // Register user to verify list before starting stream
    // If this fails, return error stream instead of silently continuing
    if let Err(e) = verify_service
        .append_user_to_verify_list(
            user_id,
            Some(state.config.rss.max_rss_paper as i32),
            payload.channel.clone(),
            payload
                .max_match_limit_per_user
                .unwrap_or(state.config.rss.max_match_limit_per_user as i32),
        )
        .await
    {
        tracing::error!("Failed to append user to verify list: {}", e);
        // Return an SSE stream that immediately sends an error event and closes
        let error_message = format!("Failed to start verification: {e}");
        let error_stream = futures::stream::once(async move {
            let error_event = Event::default().event("error").data(format!(
                r#"data: {{"type":"error","message":"{error_message}"}}"#
            ));
            Ok::<Event, ApiError>(error_event)
        });
        return Sse::new(
            Box::pin(error_stream) as Pin<Box<dyn Stream<Item = Result<Event, ApiError>> + Send>>
        )
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)));
    }

    // Capture needed vars for SSE closure to avoid moving out of captured variables
    let search_params_for_sse = payload.search_params.clone().map(std::sync::Arc::new);
    let conn_clone_for_sse = state.conn.clone();

    // Use the new stream creation function from stream_verify module
    let stream = create_verify_stream(
        user_id,
        monitor,
        rx,
        verify_service,
        search_params_for_sse,
        conn_clone_for_sse,
    );

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

    let verify_service = VerifyService::new(
        state.redis.clone().pool,
        state.conn.clone(),
        state.redis.pubsub_manager.clone(),
        state.config.rss.feed_redis.redis_prefix.clone(),
        state.config.rss.feed_redis.redis_key_default_expire,
        state.config.rss.verify_papers_channel.clone(),
    )
    .await;

    // Get all user IDs from verify list
    let user_ids = verify_service.get_active_verification_users().await?;

    tracing::info!("Found {} users in verify list", user_ids.len());

    // Get verify info for each user
    let mut results = Vec::new();
    for user_id in user_ids {
        match verify_service
            .get_user_verify_statistics(user_id, None)
            .await
        {
            Ok(verify_statistics) => {
                // If this is the current user, include user info
                let user_info = if user_id == user.id {
                    Some(user.clone())
                } else {
                    None
                };
                let info = verify_statistics.verify_info;

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
