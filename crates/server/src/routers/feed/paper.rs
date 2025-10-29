use super::FEED_TAG;
use crate::{
    middlewares::auth::User,
    model::{base::ApiResponse, page::Pagination},
    state::app_state::AppState,
};
use axum::extract::{Query, State};
use common::{error::api_error::*, prelude::ApiCode};
use seaorm_db::entities::feed::user_paper_verifications::VerificationMatch;
use seaorm_db::query::feed::{
    rss_papers::RssPaperDataWithDetail,
    user_paper_verifications::{ListUnverifiedParams, UserPaperVerificationsQuery},
};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PapersRequest {
    /// Page number for pagination (optional)
    pub page: Option<i32>,
    /// Number of items per page (optional)
    pub page_size: Option<i32>,
    pub channel: Option<String>,
    pub keyword: Option<String>,
    pub rss_source_id: Option<i32>,
    #[serde(default = "default_verification_match")]
    pub not_match: Option<VerificationMatch>,
}

fn default_verification_match() -> Option<VerificationMatch> {
    Some(VerificationMatch::Yes)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnverifiedPapersResponse {
    pub pagination: Pagination,
    pub papers: Vec<RssPaperDataWithDetail>,
}

#[utoipa::path(
    get,
    path = "/unverified-papers",
    summary = "Get unverified papers",
    description = r#"
Retrieve a paginated or complete list of papers that have not yet been verified against user interests.

## Overview
This endpoint returns papers from the user's RSS subscriptions that are awaiting verification. These papers have been fetched from subscribed RSS sources but have not yet been matched against the user's defined interests using AI verification.

## Query Parameters

### Pagination Parameters
⚠️ **Important Pagination Logic**: 
- **If NEITHER `page` NOR `page_size` is provided**: Returns ALL unverified papers (no pagination)
- **If EITHER `page` OR `page_size` is provided**: Uses pagination with defaults
  - `page` defaults to `1` if not provided
  - `page_size` defaults to `20` if not provided

Examples:
- No params: `GET /unverified-papers` → Returns all papers
- Page only: `GET /unverified-papers?page=2` → Returns page 2 with default 20 items
- Size only: `GET /unverified-papers?page_size=10` → Returns first 10 items
- Both provided: `GET /unverified-papers?page=1&page_size=50` → Returns first 50 items

### Filtering Parameters
- `channel` (optional): Filter papers by specific channel name (e.g., "arxiv", "default"). Only shows papers from matching channel.
- `keyword` (optional): Search keyword to filter papers by title or content. Performs substring matching.
- `rss_source_id` (optional): Filter papers by specific RSS source ID. Only shows papers from that exact source.
- `not_match` (optional, default: `Some(VerificationMatch::Yes)`): Filter papers by verification match status. Currently defaults to "yes" but can be used to filter by match type.

## Returns

Returns an `UnverifiedPapersResponse` object containing:

### Pagination Object
```json
{
  "page": 1,
  "page_size": 20,
  "total": 156,
  "total_pages": 8
}
```
When no pagination params are provided, pagination info reflects the complete dataset:
```json
{
  "page": 1,
  "page_size": 156,  // Total count
  "total": 156,
  "total_pages": 1
}
```

### Papers Array
Array of `RssPaperDataWithDetail` objects, each containing:
- **Paper Core Fields**: id, title, link, description, author, pub_date
- **Source Information**: source_id, source details
- **Metadata**: created_at, updated_at
- **Additional Fields**: Category tags, content preview, etc.

### Example Paper Object
```json
{
  "id": 12345,
  "title": "Example Paper Title",
  "link": "https://arxiv.org/abs/2401.12345",
  "description": "Paper abstract or description...",
  "author": "John Doe, Jane Smith",
  "pub_date": "2024-01-01T00:00:00Z",
  "source_id": 42,
  "source": {
    "id": 42,
    "name": "AI Research|Machine Learning",
    "channel": "arxiv",
    "url": "https://arxiv.org/feed",
    "logo_img": "https://example.com/logo.png"
  }
}
```

## Example Requests

### Get All Unverified Papers (No Pagination)
```
GET /unverified-papers
```
Returns every unverified paper for the user.

### Paginated Request
```
GET /unverified-papers?page=1&page_size=50
```
Returns first 50 papers.

### Filter by Channel
```
GET /unverified-papers?channel=arxiv
```
Returns all arxiv papers awaiting verification.

### Search by Keyword
```
GET /unverified-papers?keyword=neural%20networks
```
Returns papers containing "neural networks" in title or content.

### Filter by Source
```
GET /unverified-papers?rss_source_id=42
```
Returns papers from RSS source ID 42.

### Combined Filters
```
GET /unverified-papers?channel=arxiv&keyword=machine%20learning&page=1&page_size=100
```
Returns first 100 arxiv papers containing "machine learning".

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
        "id": 12345,
        "title": "Deep Learning for Natural Language Processing",
        "link": "https://arxiv.org/abs/2401.12345",
        "description": "This paper introduces...",
        "author": "Jane Doe, John Smith",
        "pub_date": "2024-01-15T10:00:00Z",
        "source_id": 42,
        "channel": "arxiv"
      }
    ]
  }
}
```

## Use Cases
- Display papers awaiting verification in UI
- Show new content from RSS feeds (not yet verified)
- Review papers before triggering verification
- Filter and search unverified papers
- Batch verification preparation
- Export unverified papers list
- Channel-specific paper browsing

## Paper Verification Workflow

1. **User subscribes to RSS sources** (via `/subscriptions` endpoint)
2. **System fetches papers from RSS feeds** (background process)
3. **Papers appear in unverified list** (this endpoint)
4. **User can review papers** (browse, filter, search)
5. **User triggers verification** (via `POST /verify` endpoint)
6. **System verifies papers against interests** (AI-powered matching)
7. **Verified papers move to verified list** (via `GET /all-verified-papers`)

## Important Notes
- These papers have NOT been verified yet (no match scores or interest mappings)
- Papers come from user's subscribed RSS sources only
- Empty results don't necessarily mean no papers exist (may be filtered out)
- Pagination defaults to ALL data if no params provided (use carefully for large datasets)
- The `not_match` parameter behavior may vary and should be tested

## Related Endpoints
- Use `GET /all-verified-papers` to see verified papers
- Use `POST /verify` to trigger verification of these papers
- Use `GET /unverified-count-info` to get count statistics
- Use `GET /unread-count` to get count of unread verified papers
"#,
    request_body = PapersRequest,
    responses(
        (status = 200, body = UnverifiedPapersResponse, description = "Successfully retrieved unverified papers with pagination"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error"),
    ),
    tag = FEED_TAG,
)]
pub async fn unverified_papers(
    State(state): State<AppState>,
    User(user): User,
    Query(payload): Query<PapersRequest>,
) -> Result<ApiResponse<UnverifiedPapersResponse>, ApiError> {
    tracing::info!("get papers");

    // Check if pagination parameters are provided
    let use_pagination = payload.page.is_some() || payload.page_size.is_some();

    // If pagination parameters are provided, use pagination; otherwise return all data
    let (offset, limit) = if use_pagination {
        let page = payload.page.unwrap_or(1);
        let page_size = payload.page_size.unwrap_or(20);
        let offset = i32::max(page - 1, 0) * page_size;
        (Some(offset), Some(page_size))
    } else {
        (None, None)
    };

    let unverified_result = UserPaperVerificationsQuery::list_unverified_papers(
        &state.conn,
        user.id,
        ListUnverifiedParams {
            offset,
            limit,
            channel: payload.channel.clone(),
            keyword: payload.keyword.clone(),
        },
    )
    .await
    .context(DbErrSnafu {
        stage: "list-unverified-papers",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;

    let (rss_papers, total) = (unverified_result.items, unverified_result.total);

    // Set response based on whether pagination is used
    let pagination = if use_pagination {
        let page = payload.page.unwrap_or(1);
        let page_size = payload.page_size.unwrap_or(20);
        Pagination {
            page,
            page_size,
            total,
            total_pages: if page_size > 0 {
                total / page_size as u64
            } else {
                0
            },
        }
    } else {
        // When not using pagination, return pagination info for all data
        Pagination {
            page: 1,
            page_size: total as i32,
            total,
            total_pages: 1,
        }
    };

    Ok(ApiResponse::data(UnverifiedPapersResponse {
        pagination,
        papers: rss_papers,
    }))
}
