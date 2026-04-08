//! File upload / download handlers — `/api/v1/namespaces/:ns/apps/:app/files`
//!
//! Upload protocol
//! ---------------
//! `PUT /api/v1/namespaces/:ns/apps/:app/files/:filename`
//!
//! - Body: raw file bytes (`.fasm` source or `.fasmc` bytecode)
//! - `Content-Encoding: gzip` → server decompresses before storing
//! - Auto-detection: if bytes start with `FSMC` magic → treated as bytecode,
//!   otherwise treated as UTF-8 FASM source

use std::io::Read;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use flate2::read::GzDecoder;

use super::auth::require_auth;
use crate::http_handler::AppState;

// ── list ──────────────────────────────────────────────────────────────────────

/// `GET /api/v1/namespaces/:ns/apps/:app/files`
pub async fn list_files(
    State(state): State<AppState>,
    Path((ns, app)): Path<(String, String)>,
) -> Response {
    match state.registry.get_app(&ns, &app).await {
        Some(manifest) => (StatusCode::OK, Json(manifest.files)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            format!("app '{}/{}' not found", ns, app),
        )
            .into_response(),
    }
}

// ── upload ────────────────────────────────────────────────────────────────────

/// `PUT /api/v1/namespaces/:ns/apps/:app/files/:filename`
pub async fn upload_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((ns, app, filename)): Path<(String, String, String)>,
    body: Bytes,
) -> Response {
    if let Err(r) = require_auth(&headers, &state) {
        return *r;
    }

    // Decompress if Content-Encoding: gzip
    let raw: Vec<u8> = if is_gzip(&headers) {
        match decompress_gzip(&body) {
            Ok(b) => b,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("gzip decompress failed: {}", e),
                )
                    .into_response()
            }
        }
    } else {
        body.to_vec()
    };

    if raw.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty file body").into_response();
    }

    match state.registry.store_file(&ns, &app, &filename, &raw).await {
        Ok(record) => (StatusCode::OK, Json(record)).into_response(),
        Err(e) if e.contains("not found") => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

// ── download ──────────────────────────────────────────────────────────────────

/// `GET /api/v1/namespaces/:ns/apps/:app/files/:filename`
pub async fn download_file(
    State(state): State<AppState>,
    Path((ns, app, filename)): Path<(String, String, String)>,
) -> Response {
    match state.registry.get_file_path(&ns, &app, &filename).await {
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => {
                let ct = if filename.ends_with(".fasmc") {
                    "application/octet-stream"
                } else {
                    "text/plain; charset=utf-8"
                };
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, ct)],
                    bytes,
                )
                    .into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        None => (
            StatusCode::NOT_FOUND,
            format!("file '{}' not found", filename),
        )
            .into_response(),
    }
}

// ── delete ────────────────────────────────────────────────────────────────────

/// `DELETE /api/v1/namespaces/:ns/apps/:app/files/:filename`
pub async fn delete_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((ns, app, filename)): Path<(String, String, String)>,
) -> Response {
    if let Err(r) = require_auth(&headers, &state) {
        return *r;
    }
    match state.registry.delete_file(&ns, &app, &filename).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e.contains("not found") => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) if e.contains("referenced") => (StatusCode::CONFLICT, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn is_gzip(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("gzip"))
        .unwrap_or(false)
}

fn decompress_gzip(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}
