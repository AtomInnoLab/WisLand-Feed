use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, State};
use common::{error::api_error::*, prelude::ApiCode};
use seaorm_db::{
    entities::feed::rss_sources,
    query::feed::{
        rss_sources::{RssSourceData, RssSourcesQuery},
        rss_subscriptions::RssSubscriptionsQuery,
    },
};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use utoipa::ToSchema;

use crate::{middlewares::auth::User, model::base::ApiResponse, state::app_state::AppState};

use super::FEED_TAG;

#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum RssNode {
    Leaf(Box<rss_sources::Model>),
    Branch(Box<RssTree>),
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RssTree {
    pub name: String,
    pub children: BTreeMap<String, RssNode>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RssTreeVec {
    pub name: String,
    #[schema(no_recursion)]
    pub children: Vec<RssTreeVec>,
    pub data: Option<rss_sources::Model>,
}

pub fn convert_to_tree(rss_sources: Vec<rss_sources::Model>) -> RssTree {
    let mut tree = RssTree {
        name: "root".to_string(),
        children: BTreeMap::new(),
    };
    for rss_source in rss_sources {
        let mut levels = vec![rss_source.channel.clone()];
        levels.extend(rss_source.name.split('|').map(|s| s.to_string()));
        let mut current_tree = &mut tree;
        for (i, level) in levels.iter().enumerate() {
            if i == levels.len() - 1 {
                // Last level, insert Leaf directly
                current_tree
                    .children
                    .insert(level.to_string(), RssNode::Leaf(Box::new(rss_source)));
                break;
            } else {
                // Intermediate level, create Branch
                if !current_tree.children.contains_key(level) {
                    current_tree.children.insert(
                        level.to_string(),
                        RssNode::Branch(Box::new(RssTree {
                            name: level.to_string(),
                            children: BTreeMap::new(),
                        })),
                    );
                }
                current_tree = match current_tree.children.get_mut(level).unwrap() {
                    RssNode::Branch(branch) => branch.as_mut(),
                    RssNode::Leaf(_) => unreachable!(),
                };
            }
        }
    }
    tree
}

pub fn convert_btreemap_to_vec(tree: RssTree) -> RssTreeVec {
    let mut children_vec = Vec::new();

    for (key, node) in tree.children {
        let child_tree = match node {
            RssNode::Leaf(data) => RssTreeVec {
                name: key,
                data: Some((*data).clone()),
                children: vec![],
            },
            RssNode::Branch(branch_tree) => {
                let converted_tree = convert_btreemap_to_vec(*branch_tree);
                RssTreeVec {
                    name: key,
                    children: converted_tree.children,
                    data: None,
                }
            }
        };
        children_vec.push(child_tree);
    }

    RssTreeVec {
        name: tree.name,
        children: children_vec,
        data: None,
    }
}

#[utoipa::path(
    get,
    path = "/rss",
    summary = "Get all RSS sources in tree structure",
    description = r#"
Retrieve all available RSS sources organized in a hierarchical tree structure.

## Overview
This endpoint returns all RSS sources from the system, organized into a tree structure based on their channel and name hierarchy.

## Tree Structure
The RSS sources are organized hierarchically:
- Root level: Channels (e.g., "default", "academic", "news")
- Sub-levels: Categories separated by pipe characters in the RSS source name
- Leaf nodes: Individual RSS sources with their full metadata

## Returns
Returns a `RssTreeVec` object containing:
- `name`: Node name
- `children`: Array of child nodes (recursive structure)
- `data`: RSS source details (only present for leaf nodes)
  - id, channel, name, url, description
  - logo_img, background_img
  - created_at, updated_at, last_fetched_at

## Use Cases
- Display RSS sources in a hierarchical UI
- Browse available sources by category
- Show the complete RSS source catalog
"#,
    responses(
        (status = 200, body = RssTreeVec, description = "Successfully retrieved RSS sources tree structure"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error"),
    ),
    tag = FEED_TAG,
)]
pub async fn rss(
    State(state): State<AppState>,
    User(_user): User,
) -> Result<ApiResponse<RssTreeVec>, ApiError> {
    tracing::info!("list rss sources");

    let rss_sources = RssSourcesQuery::list_all(&state.conn, Some("arxiv"))
        .await
        .context(DbErrSnafu {
            stage: "list-rss-sources",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    let tree = convert_to_tree(rss_sources);
    let tree_vec = convert_btreemap_to_vec(tree);
    Ok(ApiResponse::data(tree_vec))
    // Ok(ApiResponse::data(rss_sources))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserRssResponse {
    pub source_map: Vec<rss_sources::Model>,
}

#[utoipa::path(
    get,
    path = "/user_rss",
    summary = "Get user's subscribed RSS sources",
    description = r#"
Retrieve all RSS sources that the authenticated user has subscribed to.

## Overview
This endpoint returns a list of RSS sources that the current user is subscribed to, based on their subscription records.

## Returns
Returns a `UserRssResponse` object containing:
- `source_map`: Array of RSS source models with complete metadata
  - Deduplicated list of sources
  - Each source includes: id, channel, name, url, description, logo_img, background_img, timestamps

## Use Cases
- Display user's subscribed feeds
- Show personalized RSS source list
- Filter papers by user's subscriptions
- Manage user's feed preferences

## Note
The returned sources are automatically deduplicated, so each unique source appears only once even if the user has multiple subscriptions to it.
"#,
    responses(
        (status = 200, body = UserRssResponse, description = "Successfully retrieved user's subscribed RSS sources"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 500, description = "Database error"),
    ),
    tag = FEED_TAG,
)]
pub async fn user_rss(
    State(state): State<AppState>,
    User(user): User,
) -> Result<ApiResponse<UserRssResponse>, ApiError> {
    tracing::info!(user_id = user.id, "list user subscribed rss sources");

    let subscriptions = RssSubscriptionsQuery::list_by_user_id(&state.conn, user.id, None)
        .await
        .context(DbErrSnafu {
            stage: "get-rss-subscriptions",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    let mut source_ids: Vec<i32> = subscriptions.into_iter().map(|s| s.source_id).collect();
    source_ids.sort_unstable();
    source_ids.dedup();

    let source_map: Vec<rss_sources::Model> = if source_ids.is_empty() {
        Vec::new()
    } else {
        RssSourcesQuery::get_by_ids(&state.conn, source_ids)
            .await
            .context(DbErrSnafu {
                stage: "get-rss-sources",
                code: ApiCode::COMMON_DATABASE_ERROR,
            })?
    };

    Ok(ApiResponse::data(UserRssResponse { source_map }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRssSource {
    pub channel: String,
    pub name: String,
    pub url: String,
    pub description: Option<String>,
    pub logo_img: Option<String>,
    pub background_img: Option<String>,
}

#[utoipa::path(
    get,
    path = "/rss/{id}",
    summary = "Get RSS source details by ID",
    description = r#"
Retrieve detailed information about a specific RSS source.

## Overview
This endpoint returns complete metadata for a single RSS source identified by its ID.

## Parameters
- `id`: The unique identifier of the RSS source

## Returns
Returns an `rss_sources::Model` object containing:
- `id`: Unique identifier
- `channel`: Channel name (e.g., "default", "academic")
- `name`: RSS source name (may contain hierarchy with pipe separators)
- `url`: RSS feed URL
- `description`: Optional description text
- `logo_img`: Optional logo image URL
- `background_img`: Optional background image URL
- `created_at`: Creation timestamp
- `updated_at`: Last update timestamp
- `last_fetched_at`: Timestamp of last successful feed fetch

## Use Cases
- Display RSS source details page
- Edit RSS source information
- Show feed metadata before subscribing
"#,
    params(
        ("id" = i32, Path, description = "The unique identifier of the RSS source to retrieve"),
    ),
    responses(
        (status = 200, body = rss_sources::Model, description = "Successfully retrieved RSS source details"),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 404, description = "RSS source not found"),
        (status = 500, description = "Database error"),
    ),
    tag = FEED_TAG,
)]
pub async fn rss_detail(
    Path(id): Path<i32>,
    State(state): State<AppState>,
    User(_user): User,
) -> Result<ApiResponse<rss_sources::Model>, ApiError> {
    tracing::info!(id, "get rss source detail");

    let item = RssSourcesQuery::get_by_id(&state.conn, id)
        .await
        .context(DbErrSnafu {
            stage: "get-rss-source",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    Ok(ApiResponse::data(item))
}

#[utoipa::path(
    post,
    path = "/rss",
    summary = "Create a new RSS source",
    description = r#"
Create a new RSS source in the system.

## Overview
This endpoint allows adding a new RSS feed source to the system. The source will be available for users to subscribe to.

## Request Body
```json
{
  "channel": "academic",
  "name": "AI Research|Machine Learning",
  "url": "https://example.com/feed.xml",
  "description": "Latest machine learning research papers",
  "logo_img": "https://example.com/logo.png",
  "background_img": "https://example.com/bg.jpg"
}
```

## Fields
- `channel` (required): Channel category for organizing sources
- `name` (required): Source name, can use pipe (|) for hierarchy
- `url` (required): RSS feed URL
- `description` (optional): Descriptive text about the source
- `logo_img` (optional): Logo image URL
- `background_img` (optional): Background image URL

## Returns
Returns the `id` (i32) of the newly created RSS source.

## Use Cases
- Add new RSS feeds to the system
- Create custom feed categories
- Expand available content sources

## Note
The `name` field supports hierarchical organization using pipe separators (e.g., "Category|Subcategory|Feed Name").
"#,
    request_body = CreateRssSource,
    responses(
        (status = 200, description = "RSS source created successfully, returns the new source ID", body = i32),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 400, description = "Invalid request data"),
        (status = 500, description = "Database error or creation failed"),
    ),
    tag = FEED_TAG,
)]
pub async fn rss_create(
    State(state): State<AppState>,
    User(_user): User,
    Json(payload): Json<CreateRssSource>,
) -> Result<ApiResponse<i32>, ApiError> {
    tracing::info!(name = payload.name, url = payload.url, "create rss source");

    let id = RssSourcesQuery::insert(
        &state.conn,
        RssSourceData {
            id: None,
            channel: payload.channel,
            name: payload.name,
            url: payload.url,
            description: payload.description,
            logo_img: payload.logo_img,
            background_img: payload.background_img,
            last_fetched_at: None,
        },
    )
    .await
    .context(DbErrSnafu {
        stage: "create-rss-source",
        code: ApiCode::COMMON_DATABASE_ERROR,
    })?;

    Ok(ApiResponse::data(id))
}

#[utoipa::path(
    delete,
    path = "/rss/{id}",
    summary = "Delete an RSS source",
    description = r#"
Delete an RSS source from the system.

## Overview
This endpoint permanently removes an RSS source from the database.

## Parameters
- `id`: The unique identifier of the RSS source to delete

## Returns
Returns `true` if the deletion was successful.

## Side Effects
- The RSS source will be permanently removed
- Any subscriptions to this source may be affected
- Historical papers from this source are typically preserved

## Use Cases
- Remove outdated or inactive feeds
- Clean up duplicate sources
- Maintain RSS source catalog

## Warning
This operation is permanent and cannot be undone. Ensure the correct ID is specified before deletion.
"#,
    params(
        ("id" = i32, Path, description = "The unique identifier of the RSS source to delete"),
    ),
    responses(
        (status = 200, description = "RSS source deleted successfully, returns true", body = bool),
        (status = 401, description = "Unauthorized - valid authentication required"),
        (status = 404, description = "RSS source not found"),
        (status = 500, description = "Database error or deletion failed"),
    ),
    tag = FEED_TAG,
)]
pub async fn rss_delete(
    Path(id): Path<i32>,
    State(state): State<AppState>,
    User(_user): User,
) -> Result<ApiResponse<bool>, ApiError> {
    tracing::info!(id, "delete rss source");

    RssSourcesQuery::delete_by_id(&state.conn, id)
        .await
        .context(DbErrSnafu {
            stage: "delete-rss-source",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    Ok(ApiResponse::data(true))
}
