use axum::extract::FromRequestParts;
use common::{error::api_error::*, prelude::ApiCode};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use tracing::info;
use utoipa::ToSchema;

use crate::consts::{WIS_TOKEN, WIS_TOKEN_LOWERCASE};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserInfo {
    pub id: i64,
    pub open_id: String,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub nickname: Option<String>,
    pub preferred_username: Option<String>,
    pub profile: Option<String>,
    pub picture: Option<String>,
    pub website: Option<String>,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub gender: Option<String>,
    pub birthdate: Option<String>,
    pub zoneinfo: Option<String>,
    pub locale: Option<String>,
    pub phone_number: Option<String>,
    pub phone_number_verified: Option<bool>,
    pub address: Option<String>,
}

pub struct User(pub UserInfo);

impl<S> FromRequestParts<S> for User {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let headers = &mut parts.headers;
        let wis_token = headers
            .remove(WIS_TOKEN)
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

            serde_json::from_str::<UserInfo>(user)
                .context(SerializeSnafu {
                    stage: "deserialize-auth-user",
                    code: ApiCode::INVALID_AUTH_PAYLOAD,
                })
                .map(User)
        }
    }
}
