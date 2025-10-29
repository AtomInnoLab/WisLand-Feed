use axum::response::IntoResponse;
use common::{error::api_error::ApiError, prelude::ApiCode};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::app_state::AppState;

#[utoipa::path(
    get,
    path = "/health",
    summary = "Health check endpoint",
    description = r#"
Check the health status of the API server.

## Overview
This is a simple health check endpoint that returns the string "ok" to indicate the server is running and responding to requests. This endpoint is commonly used by monitoring systems, load balancers, and deployment tools to verify server availability.

## Response
Returns the plain text string "ok" (HTTP 200 OK).

**Response Format:**
```
ok
```

## Behavior
- **No Authentication Required**: This endpoint does not require authentication tokens
- **No Parameters**: No query parameters or request body needed
- **Always Returns Success**: If the server is running and can respond, it returns "ok"
- **Content Type**: Returns `text/plain` content type
- **Status Code**: Always returns HTTP 200 OK

## Use Cases
- Load balancer health checks
- Kubernetes liveness/readiness probes
- Monitoring system uptime checks
- Deployment verification
- Basic connectivity testing

## Important Notes
- This endpoint does NOT verify database connectivity
- This endpoint does NOT verify Redis connectivity
- This endpoint does NOT verify system resource availability
- It only confirms the HTTP server is running and can respond to requests

## Monitoring Recommendations
For comprehensive health monitoring, consider checking additional endpoints:
- Database connectivity: Monitor response times from authenticated endpoints
- Redis connectivity: Monitor verification endpoints
- System resources: Use infrastructure monitoring tools
"#,
    responses(
        (status = 200, description = "Server is healthy and responding", body = String, content_type = "text/plain"),
        (status = 500, description = "Server is not responding (unlikely to reach this if server is down)"),
    ),
    tag = "Common"
)]
pub async fn health() -> &'static str {
    "ok"
}

pub fn health_routers() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(health))
}

/// 404 handler
pub async fn handler_404() -> impl IntoResponse {
    ApiError::NotFound {
        code: ApiCode {
            http_code: 404,
            code: 200000,
        },
    }
}
