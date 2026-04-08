//! Route registration / unregistration handlers.
//!
//! `POST /api/v1/namespaces/:ns/apps/:app/routes`
//!   → compiles the target file, inserts into the live RouteTable, persists manifest.
//!
//! `DELETE /api/v1/namespaces/:ns/apps/:app/routes/:route_id`
//!   → removes from RouteTable + manifest immediately (in-flight reqs finish normally).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use fasm_bytecode::decode_program;
use fasm_compiler::compile_source;
use serde::Deserialize;
use uuid::Uuid;

use super::{auth::require_auth, registry::RouteRecord};
use crate::http_handler::AppState;

#[derive(Deserialize)]
pub struct RegisterRouteBody {
    pub method: String,
    pub path: String,
    pub function: String,
    pub file: String,
}

// ── list ──────────────────────────────────────────────────────────────────────

/// `GET /api/v1/namespaces/:ns/apps/:app/routes`
pub async fn list_routes(
    State(state): State<AppState>,
    Path((ns, app)): Path<(String, String)>,
) -> Response {
    match state.registry.get_app(&ns, &app).await {
        Some(manifest) => (StatusCode::OK, Json(manifest.routes)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            format!("app '{}/{}' not found", ns, app),
        )
            .into_response(),
    }
}

// ── register ──────────────────────────────────────────────────────────────────

/// `POST /api/v1/namespaces/:ns/apps/:app/routes`
///
/// Compiles (or decodes) the referenced file and hot-inserts the route.
pub async fn register_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((ns, app)): Path<(String, String)>,
    Json(body): Json<RegisterRouteBody>,
) -> Response {
    if let Err(r) = require_auth(&headers, &state) {
        return r;
    }

    // Verify the app exists and the file is uploaded.
    let manifest = match state.registry.get_app(&ns, &app).await {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                format!("app '{}/{}' not found", ns, app),
            )
                .into_response()
        }
    };
    if !manifest.files.iter().any(|f| f.name == body.file) {
        return (
            StatusCode::NOT_FOUND,
            format!("file '{}' not uploaded to app '{}/{}'", body.file, ns, app),
        )
            .into_response();
    }

    // Load and compile/decode the file.
    let file_path = state.registry.file_path(&ns, &app, &body.file);
    let program = match load_program(&file_path) {
        Ok(p) => Arc::new(p),
        Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    };

    // Insert into the live RouteTable.
    let route_id = {
        let mut routes = state.routes.write().await;
        match routes.add_route_dyn(&body.method, &body.path, body.function.clone(), program) {
            Ok(id) => id,
            Err(e) => return (StatusCode::CONFLICT, e).into_response(),
        }
    };

    // Persist the record in the manifest.
    let record = RouteRecord {
        id: route_id,
        method: body.method.to_uppercase(),
        path: body.path.clone(),
        function: body.function.clone(),
        file: body.file.clone(),
    };
    if let Err(e) = state
        .registry
        .add_route_record(&ns, &app, record.clone())
        .await
    {
        // Roll back the RouteTable insertion so state stays consistent.
        let mut routes = state.routes.write().await;
        routes.remove_route(route_id);
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }

    (StatusCode::CREATED, Json(record)).into_response()
}

// ── unregister ────────────────────────────────────────────────────────────────

/// `DELETE /api/v1/namespaces/:ns/apps/:app/routes/:route_id`
pub async fn unregister_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((ns, app, route_id_str)): Path<(String, String, String)>,
) -> Response {
    if let Err(r) = require_auth(&headers, &state) {
        return r;
    }

    let route_id = match Uuid::parse_str(&route_id_str) {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "invalid route_id (expected UUID)").into_response()
        }
    };

    // Remove from live RouteTable first.
    let removed = {
        let mut routes = state.routes.write().await;
        routes.remove_route(route_id)
    };

    if !removed {
        return (
            StatusCode::NOT_FOUND,
            format!("route '{}' not found or is a static route", route_id),
        )
            .into_response();
    }

    // Remove from persisted manifest.
    if let Err(e) = state
        .registry
        .remove_route_record(&ns, &app, route_id)
        .await
    {
        tracing::warn!("route removed from table but manifest update failed: {}", e);
    }

    StatusCode::NO_CONTENT.into_response()
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn load_program(path: &std::path::Path) -> Result<fasm_bytecode::Program, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read file {:?}: {}", path, e))?;

    if bytes.starts_with(b"FSMC") {
        // Pre-compiled bytecode
        decode_program(&bytes).map_err(|e| format!("bytecode decode error: {}", e))
    } else {
        // FASM source text
        let src = std::str::from_utf8(&bytes)
            .map_err(|_| "file is not valid UTF-8 FASM source".to_string())?;
        compile_source(src).map_err(|e| format!("compile error: {}", e))
    }
}
