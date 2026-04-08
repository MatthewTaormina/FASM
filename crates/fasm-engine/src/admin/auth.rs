//! Auth helper — checks `X-Admin-Token` header against config.

use crate::http_handler::AppState;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

/// Returns `Ok(())` if auth passes, `Err(Response)` (401) if it fails.
pub fn require_auth(headers: &HeaderMap, state: &AppState) -> Result<(), Response> {
    let Some(expected) = &state.admin_token else {
        return Ok(()); // no token configured → open
    };
    let provided = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided == expected {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, "invalid or missing X-Admin-Token").into_response())
    }
}
