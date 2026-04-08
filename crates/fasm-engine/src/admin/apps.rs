//! App CRUD handlers — `/api/v1/namespaces/:ns/apps`

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

use crate::http_handler::AppState;
use super::auth::require_auth;

#[derive(Deserialize)]
pub struct CreateAppBody {
    pub name: String,
}

/// `GET /api/v1/namespaces/:ns/apps`
pub async fn list_apps(
    State(state): State<AppState>,
    Path(ns): Path<String>,
) -> Response {
    match state.registry.list_apps(&ns).await {
        Ok(apps) => (StatusCode::OK, Json(apps)).into_response(),
        Err(e)   => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// `POST /api/v1/namespaces/:ns/apps`
pub async fn create_app(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(ns): Path<String>,
    Json(body): Json<CreateAppBody>,
) -> Response {
    if let Err(r) = require_auth(&headers, &state) { return r; }
    match state.registry.create_app(&ns, &body.name).await {
        Ok(manifest)  => (StatusCode::CREATED, Json(manifest)).into_response(),
        Err(e) if e.contains("not found") => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) if e.contains("already exists") => (StatusCode::CONFLICT, e).into_response(),
        Err(e)        => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// `GET /api/v1/namespaces/:ns/apps/:app`
pub async fn get_app(
    State(state): State<AppState>,
    Path((ns, app)): Path<(String, String)>,
) -> Response {
    match state.registry.get_app(&ns, &app).await {
        Some(manifest) => (StatusCode::OK, Json(manifest)).into_response(),
        None           => (StatusCode::NOT_FOUND, format!("app '{}/{}' not found", ns, app)).into_response(),
    }
}

/// `DELETE /api/v1/namespaces/:ns/apps/:app`
///
/// Unloads all routes belonging to this app before deleting.
pub async fn delete_app(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((ns, app)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_auth(&headers, &state) { return r; }

    // Collect route IDs to unload before the manifest is deleted.
    let route_ids: Vec<uuid::Uuid> = if let Some(manifest) = state.registry.get_app(&ns, &app).await {
        manifest.routes.iter().map(|r| r.id).collect()
    } else {
        return (StatusCode::NOT_FOUND, format!("app '{}/{}' not found", ns, app)).into_response();
    };

    // Hot-unload all routes from the live RouteTable.
    {
        let mut routes = state.routes.write().await;
        for id in route_ids {
            routes.remove_route(id);
        }
    }

    match state.registry.delete_app(&ns, &app).await {
        Ok(())  => StatusCode::NO_CONTENT.into_response(),
        Err(e)  => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
