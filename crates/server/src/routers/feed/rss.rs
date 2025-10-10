use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, State};
use common::{error::api_error::*, prelude::ApiCode};
use seaorm_db::{
    entities::feed::rss_sources,
    query::feed::rss_sources::{RssSourceData, RssSourcesQuery},
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
    responses(
        (status = 200, body = RssTreeVec),
    ),
    tag = FEED_TAG,
)]
pub async fn rss(
    State(state): State<AppState>,
    User(_user): User,
) -> Result<ApiResponse<RssTreeVec>, ApiError> {
    tracing::info!("list rss sources");

    let rss_sources = RssSourcesQuery::list_all(&state.conn)
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
    params(
        ("id" = i32, Path, description = "RSS source ID"),
    ),
    responses(
        (status = 200, body = rss_sources::Model),
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
    request_body = CreateRssSource,
    responses(
        (status = 200, description = "Created successfully, returns new ID", body = i32),
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
    params(
        ("id" = i32, Path, description = "RSS source ID"),
    ),
    responses(
        (status = 200, description = "Deleted successfully", body = bool),
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
