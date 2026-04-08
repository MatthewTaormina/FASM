//! Axum HTTP handler.
//!
//! A single wildcard route catches all traffic and dispatches to `RouteTable`.
//!
//! ## `$args` struct layout
//!
//! The FASM `$args` struct passed to each handler function is built from three
//! input sources, packed in order with sequential `u32` field keys:
//!
//! | Field key | Source | FASM type |
//! |-----------|--------|-----------|
//! | 0, 1, … | Path params (`:id`, `:slug`, …) | `VEC<UINT8>` — UTF-8 bytes |
//! | next | Query params (`?k=v`) — one slot per `k=v` pair | `VEC<UINT8>` — UTF-8 bytes |
//! | last | Request body | See below |
//!
//! ### Body type mapping
//!
//! | HTTP body | FASM value |
//! |-----------|------------|
//! | `application/json` object `{"a":1}` | `STRUCT { 0→INT32(1) }` — string keys discarded, positional |
//! | `application/json` array `[1,"hi"]` | `VEC [ INT32(1), VEC<UINT8>("hi") ]` |
//! | `application/json` string `"hello"` | `VEC<UINT8>` bytes of `hello` |
//! | `application/json` number `42` | `INT32` (or `INT64` if >i32::MAX) |
//! | `application/json` float `3.14` | `FLOAT64` |
//! | `application/json` bool | `BOOL` |
//! | `application/json` null | `NULL` |
//! | Any other content-type | `VEC<UINT8>` raw bytes |

use axum::{
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use fasm_vm::{
    value::{FasmStruct, FasmVec},
    Value,
};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{
    dispatcher::{EngineError, ExecRequest, TaskDispatcher},
    metrics::MetricsRegistry,
    router::RouteTable,
};

// ── AppState ──────────────────────────────────────────────────────────────────

/// Shared state threaded through axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub routes: Arc<RwLock<RouteTable>>,
    pub dispatcher: TaskDispatcher,
    pub metrics: MetricsRegistry,
    /// Optional token required on all `/api/v1/` requests.
    pub admin_token: Option<String>,
    /// Namespace/app/file registry for the management API.
    pub registry: crate::admin::AppRegistry,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// Generic catch-all handler.  Axum does not directly expose a Router-less
/// catch-all, so callers should register this via `.fallback(handle_request)`.
pub async fn handle_request(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
) -> Response {
    let method = req.method().as_str().to_uppercase();
    let uri = req.uri().clone();
    let path = uri.path().to_string();

    // match route — hold read lock only for the lookup, then release
    let matched = {
        let routes = state.routes.read().await;
        routes.match_route(&method, &path)
    };
    let matched = match matched {
        Some(m) => m,
        None => return (StatusCode::NOT_FOUND, "404 not found").into_response(),
    };

    // Build $args struct
    let mut args_struct = FasmStruct::default();
    let mut field_idx: u32 = 0;

    // ── Path params (ordered by pattern position, always VEC<UINT8>) ───────────
    for val in matched.params.values() {
        let bytes: Vec<Value> = val.bytes().map(Value::Uint8).collect();
        args_struct.insert(field_idx, Value::Vec(FasmVec(bytes)));
        field_idx += 1;
    }

    // ── Query string params (each value as VEC<UINT8>) ────────────────────────
    if let Some(query_str) = uri.query() {
        for pair in query_str.split('&') {
            if let Some((_, v)) = pair.split_once('=') {
                // URL-decode percent-encoded chars
                let decoded = percent_decode(v);
                let bytes: Vec<Value> = decoded.bytes().map(Value::Uint8).collect();
                args_struct.insert(field_idx, Value::Vec(FasmVec(bytes)));
                field_idx += 1;
            }
        }
    }

    // ── Body: JSON → FASM value tree; anything else → VEC<UINT8> raw bytes ────
    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 1_048_576).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "request body too large or unreadable",
            )
                .into_response()
        }
    };

    if !body_bytes.is_empty() {
        let is_json = parts
            .headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.starts_with("application/json"))
            .unwrap_or(false);

        let body_val = if is_json {
            match serde_json::from_slice::<JsonValue>(&body_bytes) {
                Ok(jv) => json_to_value(jv),
                Err(e) => {
                    return (StatusCode::BAD_REQUEST, format!("invalid JSON body: {}", e))
                        .into_response()
                }
            }
        } else {
            // Raw bytes (form data, binary, plain text, …)
            Value::Vec(FasmVec(
                body_bytes.iter().map(|b| Value::Uint8(*b)).collect(),
            ))
        };

        args_struct.insert(field_idx, body_val);
    }

    // Build ExecRequest
    let exec_req = ExecRequest {
        func: matched.func.clone(),
        program: matched.program.clone(),
        args: Value::Struct(args_struct),
        trigger: "http".to_string(),
        jit: matched.jit.clone(),
    };

    // Dispatch
    match state.dispatcher.spawn_async(exec_req).await {
        Ok(ret_val) => {
            let json = value_to_json(&ret_val);
            (StatusCode::OK, Json(json)).into_response()
        }
        Err(EngineError::Overloaded) => {
            (StatusCode::SERVICE_UNAVAILABLE, "engine overloaded").into_response()
        }
        Err(EngineError::FasmFault(msg)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
        }
        Err(EngineError::JoinError(msg)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
        }
    }
}

// ── /metrics handler ──────────────────────────────────────────────────────────

pub async fn handle_metrics(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Response {
    let text = state.metrics.render_text();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        text,
    )
        .into_response()
}

// ── /admin/queues handler ─────────────────────────────────────────────────────

pub async fn handle_queue_info(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Response {
    let snap = state.metrics.snapshot();
    let json = serde_json::to_string(&snap.queue_depth).unwrap_or_default();
    (StatusCode::OK, [("content-type", "application/json")], json).into_response()
}

// ── JSON → Value conversion ─────────────────────────────────────────────────
//
// Maps a `serde_json::Value` tree to the FASM `Value` type system.
//
// Sentinels:
//   {"$b64": "<base64>"}  →  VEC<UINT8>  (binary data emitted by value_to_json)
//
// Object keys are otherwise discarded; fields become sequential u32 indices
// matching FASM's integer-keyed STRUCT model.

fn json_to_value(jv: JsonValue) -> Value {
    match jv {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Bool(b),
        JsonValue::String(s) => {
            // String literals → VEC<UINT8> (bare content, no surrounding quotes)
            Value::Vec(FasmVec(s.bytes().map(Value::Uint8).collect()))
        }
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    Value::Int32(i as i32)
                } else {
                    Value::Int64(i)
                }
            } else if let Some(f) = n.as_f64() {
                Value::Float64(f)
            } else {
                Value::Null
            }
        }
        JsonValue::Array(arr) => {
            // JSON array → VEC of recursively converted elements
            Value::Vec(FasmVec(arr.into_iter().map(json_to_value).collect()))
        }
        JsonValue::Object(obj) => {
            // Sentinel: {"$b64": "<base64>"} → VEC<UINT8> binary blob
            if obj.len() == 1 {
                if let Some(JsonValue::String(enc)) = obj.get("$b64") {
                    match B64.decode(enc) {
                        Ok(bytes) => {
                            return Value::Vec(FasmVec(
                                bytes.into_iter().map(Value::Uint8).collect(),
                            ))
                        }
                        Err(_) => { /* fall through to normal struct handling */ }
                    }
                }
            }
            // Normal object → STRUCT with positional u32 keys.
            // String keys are dropped — FASM addresses fields by integer index.
            // {"x": 1, "y": 2}  →  STRUCT { 0→INT32(1), 1→INT32(2) }
            let mut s = FasmStruct::default();
            for (idx, (_key, val)) in obj.into_iter().enumerate() {
                s.insert(idx as u32, json_to_value(val));
            }
            Value::Struct(s)
        }
    }
}

/// Minimal percent-decode for query string values (`%20` → space, etc.).
/// Only handles the `%XX` form; `+` as space is not URL-standard for paths.
fn percent_decode(s: &str) -> String {
    // Fast path: if no '%' nothing to decode
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next().and_then(|c| c.to_digit(16));
            let lo = chars.next().and_then(|c| c.to_digit(16));
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((((h << 4) | l) as u8) as char);
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ── Value → JSON conversion ───────────────────────────────────────────────────
//
// Maps a FASM `Value` tree to JSON for the HTTP response body.
//
// Symmetry rules:
//   VEC<UINT8> that is valid UTF-8  → JSON string (human-readable)
//   VEC<UINT8> that is binary       → {"$b64": "<base64-std>"}
//   VEC of other element types      → JSON array (recursed)
//   STRUCT { u32 → Value }          → JSON object {"0": …, "1": …}
//   OPTION Some(v)                  → value_to_json(v)
//   OPTION None / NULL              → null
//   RESULT Ok(v)                    → {"ok": value_to_json(v)}
//   RESULT Err(code)                → {"err": code}

fn value_to_json(v: &Value) -> JsonValue {
    match v {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Int32(n) => JsonValue::Number((*n).into()),
        Value::Int64(n) => JsonValue::Number((*n).into()),
        Value::Uint8(n) => JsonValue::Number((*n).into()),
        Value::Uint16(n) => JsonValue::Number((*n).into()),
        Value::Uint32(n) => JsonValue::Number((*n).into()),
        Value::Uint64(n) => serde_json::json!(*n),
        Value::Float32(f) => serde_json::json!(*f as f64),
        Value::Float64(f) => serde_json::json!(*f),
        Value::Vec(v) => {
            // Is this a pure UINT8 vector (byte array)?
            let bytes: Vec<u8> =
                v.0.iter()
                    .filter_map(|b| {
                        if let Value::Uint8(u) = b {
                            Some(*u)
                        } else {
                            None
                        }
                    })
                    .collect();
            if bytes.len() == v.0.len() {
                // All bytes: try UTF-8 string first (most common case)
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    return JsonValue::String(s.to_string());
                }
                // Not valid UTF-8 → base64 sentinel object
                return serde_json::json!({ "$b64": B64.encode(&bytes) });
            }
            // Mixed-type VEC → recursive JSON array
            JsonValue::Array(v.0.iter().map(value_to_json).collect())
        }
        Value::Struct(s) => {
            // STRUCT with u32 keys → JSON object with string-ified keys
            // preserves round-trip: client reads field "0", "1", … to mirror FASM access
            let mut map = serde_json::Map::new();
            for (k, val) in s.0.iter() {
                map.insert(k.to_string(), value_to_json(val));
            }
            JsonValue::Object(map)
        }
        Value::Option(opt) => match opt.as_ref() {
            fasm_vm::value::FasmOption::Some(v) => value_to_json(v),
            fasm_vm::value::FasmOption::None => JsonValue::Null,
        },
        Value::Result(r) => match r.as_ref() {
            fasm_vm::value::FasmResult::Ok(v) => serde_json::json!({"ok":  value_to_json(v)}),
            fasm_vm::value::FasmResult::Err(c) => serde_json::json!({"err": c}),
        },
        _ => JsonValue::String(format!("{:?}", v)),
    }
}
