//! Namespace CRUD handlers — `/api/v1/namespaces`

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

use super::{auth::require_auth, registry::validate_namespace};
use crate::http_handler::AppState;

#[derive(Deserialize)]
pub struct CreateNsBody {
    pub name: String,
}

/// `GET /api/v1/namespaces`
pub async fn list_namespaces(State(state): State<AppState>) -> Response {
    let list = state.registry.list_namespaces().await;
    (StatusCode::OK, Json(list)).into_response()
}

/// `POST /api/v1/namespaces`
pub async fn create_namespace(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateNsBody>,
) -> Response {
    if let Err(r) = require_auth(&headers, &state) {
        return r;
    }
    if let Err(e) = validate_namespace(&body.name) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    match state.registry.create_namespace(&body.name).await {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

/// `GET /api/v1/namespaces/:ns`
pub async fn get_namespace(State(state): State<AppState>, Path(ns): Path<String>) -> Response {
    match state.registry.get_namespace(&ns).await {
        Some(info) => (StatusCode::OK, Json(info)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            format!("namespace '{}' not found", ns),
        )
            .into_response(),
    }
}

/// `DELETE /api/v1/namespaces/:ns`
pub async fn delete_namespace(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(ns): Path<String>,
) -> Response {
    if let Err(r) = require_auth(&headers, &state) {
        return r;
    }
    match state.registry.delete_namespace(&ns).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e.contains("not found") => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) if e.contains("not empty") => (StatusCode::CONFLICT, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
