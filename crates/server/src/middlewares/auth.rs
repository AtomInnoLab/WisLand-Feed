use std::ops::Deref;

use axum::extract::FromRequestParts;
use base64::{Engine, engine::general_purpose};
use common::{
    error::api_error::{ApiError, SerializeSnafu},
    prelude::ApiCode,
};
use serde::{Deserialize, Serialize};
use tracing::info;
use utoipa::ToSchema;
use uuid::Uuid;

use snafu::ResultExt;
pub struct User(pub UserInfo);

impl Deref for User {
    type Target = UserInfo;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserInfo {
    #[serde(rename = "user_id", alias = "id")]
    pub id: i64,
    #[serde(default)]
    pub system_admin: bool,
    #[serde(default)]
    pub visitor_id: Option<Uuid>,

    /// using on website
    pub open_id: Option<String>,
    pub benefit: Option<UserBenefit>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserBenefit {
    #[default]
    Free,
    Plus,
    Pro,
}

impl UserInfo {
    #[inline(always)]
    pub fn admin_user(id: i64) -> Self {
        Self::normal_user(id, true)
    }

    #[inline(always)]
    pub fn normal_user(id: i64, admin: bool) -> Self {
        UserInfo {
            id,
            system_admin: admin,
            ..Default::default()
        }
    }

    pub fn is_free(&self) -> bool {
        self.benefit
            .is_none_or(|benefit| matches!(benefit, UserBenefit::Free))
    }
}

pub const WIS_USER_INFO: &str = "Wis-User-Info";
pub const WIS_USER_INFO_LOWERCASE: &str = "wis-user-info";
pub const WIS_TOKEN: &str = "X-User-Info";
pub const WIS_TOKEN_LOWERCASE: &str = "x-user-info";

impl<S> FromRequestParts<S> for User {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let headers = &mut parts.headers;

        let wis_token = headers
            .remove(WIS_USER_INFO)
            .or_else(|| headers.remove(WIS_USER_INFO_LOWERCASE))
            .or_else(|| headers.remove(WIS_TOKEN))
            .or_else(|| headers.remove(WIS_TOKEN_LOWERCASE));

        async move {
            let payload = wis_token.as_ref().and_then(|token| token.to_str().ok());

            let Some(user) = payload else {
                info!("No WIS token found in request headers");
                return Err(ApiError::AuthErr {
                    msg: "No Auth Token Found In Request Herders".to_string(),
                    stage: "extract-auth-header".to_string(),
                    code: ApiCode::NO_AUTH_TOKEN,
                });
            };

            match general_purpose::STANDARD.decode(user) {
                Ok(json_bytes) => serde_json::from_slice::<UserInfo>(&json_bytes)
                    .context(SerializeSnafu {
                        stage: "deserialize-auth-user",
                        code: ApiCode::INVALID_AUTH_PAYLOAD,
                    })
                    .map(User),
                Err(_) => serde_json::from_str::<UserInfo>(user)
                    .context(SerializeSnafu {
                        stage: "deserialize-auth-user",
                        code: ApiCode::INVALID_AUTH_PAYLOAD,
                    })
                    .map(User),
            }
        }
    }
}
