use axum::Json;
use axum::extract::{Path, State};
use common::{error::api_error::*, prelude::ApiCode};
use seaorm_db::{
    entities::web::feed::rss_sources,
    query::feed::rss_sources::{RssSourceData, RssSourcesQuery},
};
use serde::Deserialize;
use snafu::ResultExt;
use utoipa::ToSchema;

use crate::{middlewares::auth::User, model::base::ApiResponse, state::app_state::AppState};

use super::FEED_TAG;

#[utoipa::path(
    get,
    path = "/rss",
    responses(
        (status = 200, body = Vec<rss_sources::Model>),
    ),
    tag = FEED_TAG,
)]
pub async fn rss(
    State(state): State<AppState>,
    User(_user): User,
) -> Result<ApiResponse<Vec<rss_sources::Model>>, ApiError> {
    tracing::info!("list rss sources");

    let rss_sources = RssSourcesQuery::list_all(&state.conn)
        .await
        .context(DbErrSnafu {
            stage: "list-rss-sources",
            code: ApiCode::COMMON_DATABASE_ERROR,
        })?;

    Ok(ApiResponse::data(rss_sources))
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
        ("id" = i32, Path, description = "RSS 源 ID"),
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
        (status = 200, description = "创建成功，返回新建 ID", body = i32),
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
        ("id" = i32, Path, description = "RSS 源 ID"),
    ),
    responses(
        (status = 200, description = "删除成功", body = bool),
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
