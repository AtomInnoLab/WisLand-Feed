use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use tracing::*;

pub async fn log_response(req: Request<Body>, next: Next) -> impl IntoResponse {
    let resp = next.run(req).await.into_response();

    let status = resp.status();
    if status.is_client_error() || status.is_server_error() {
        let body = to_bytes(resp.into_body(), 250).await.unwrap_or_default();

        let body_str = String::from_utf8_lossy(&body);
        if body_str.is_empty() {
            error!(
                status = %status,
            );
        } else {
            error!(
                status = %status,
                body = %body_str,
            );
        }
        let new_resp = Response::builder()
            .status(status)
            .body(Body::from(body))
            .unwrap();
        return new_resp;
    }
    resp
}

pub async fn log_request(request: Request, next: Next) -> Result<impl IntoResponse, Response> {
    let request = buffer_request_body(request).await?;

    Ok(next.run(request).await)
}

async fn buffer_request_body(request: Request) -> Result<Request, Response> {
    let (parts, body) = request.into_parts();

    // this won't work if the body is an long running stream
    let bytes = body
        .collect()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response())?
        .to_bytes();

    if bytes.is_empty() {
        tracing::debug!(uri=?parts.uri);
    } else {
        let body = String::from_utf8_lossy(&bytes);
        tracing::debug!(uri=?parts.uri, body = body.as_ref());
    }

    Ok(Request::from_parts(parts, Body::from(bytes)))
}
