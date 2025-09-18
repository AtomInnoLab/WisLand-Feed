use axum::{http::StatusCode, response::IntoResponse};
use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct ApiResponse<T: Serialize> {
    pub data: T,
    pub success: bool,
    pub message: String,
}

impl<T> IntoResponse for ApiResponse<T>
where
    T: Serialize,
{
    fn into_response(self) -> axum::response::Response {
        (StatusCode::OK, axum::Json(self)).into_response()
    }
}

impl<T> ApiResponse<T>
where
    T: Serialize,
{
    pub fn data(data: T) -> Self {
        ApiResponse {
            data,
            success: true,
            message: "Success".to_string(),
        }
    }

    pub fn data_with_msg(data: T, msg: impl Into<String>) -> Self {
        ApiResponse {
            data,
            success: true,
            message: msg.into(),
        }
    }
}
