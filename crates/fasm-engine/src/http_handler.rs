//! Axum HTTP handler.
//!
//! A single wildcard route catches all traffic and dispatches to `RouteTable`.
//! Path params, query string, and JSON body are packed into the FASM `$args`
//! struct (field keys are sequential `u32` indices matching declaration order).

use std::sync::Arc;
use axum::{
    body::Bytes,
    extract::{Path, Query, Request},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value as JsonValue;
use fasm_vm::Value;
use fasm_vm::value::FasmStruct;

use crate::{
    dispatcher::{ExecRequest, EngineError, TaskDispatcher},
    metrics::MetricsRegistry,
    router::RouteTable,
};

// ── AppState ──────────────────────────────────────────────────────────────────

/// Shared state threaded through axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub routes:     Arc<RouteTable>,
    pub dispatcher: TaskDispatcher,
    pub metrics:    MetricsRegistry,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// Generic catch-all handler.  Axum does not directly expose a Router-less
/// catch-all, so callers should register this via `.fallback(handle_request)`.
pub async fn handle_request(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
) -> Response {
    let method  = req.method().as_str().to_uppercase();
    let uri     = req.uri().clone();
    let path    = uri.path().to_string();

    // match route
    let matched = match state.routes.match_route(&method, &path) {
        Some(m) => m,
        None    => return (StatusCode::NOT_FOUND, "404 not found").into_response(),
    };

    // Build $args struct
    let mut args_struct = FasmStruct::default();
    let mut field_idx: u32 = 0;

    // Path params (in order of match)
    for (_name, val) in &matched.params {
        let bytes: Vec<Value> = val.bytes().map(Value::Uint8).collect();
        args_struct.insert(field_idx, Value::Vec(fasm_vm::value::FasmVec(bytes)));
        field_idx += 1;
    }

    // Query string params
    if let Some(query_str) = uri.query() {
        for pair in query_str.split('&') {
            if let Some((_, v)) = pair.split_once('=') {
                let bytes: Vec<Value> = v.bytes().map(Value::Uint8).collect();
                args_struct.insert(field_idx, Value::Vec(fasm_vm::value::FasmVec(bytes)));
                field_idx += 1;
            }
        }
    }

    // Body (raw bytes) — FASM handler can decode as needed
    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, "bad request body").into_response(),
    };
    if !body_bytes.is_empty() {
        let bytes: Vec<Value> = body_bytes.iter().map(|b| Value::Uint8(*b)).collect();
        args_struct.insert(field_idx, Value::Vec(fasm_vm::value::FasmVec(bytes)));
    }

    // Build ExecRequest
    let exec_req = ExecRequest {
        func:    matched.func.clone(),
        program: matched.program.clone(),
        args:    Value::Struct(args_struct),
        trigger: "http".to_string(),
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
    (StatusCode::OK, [("content-type", "text/plain; version=0.0.4")], text).into_response()
}

// ── /admin/queues handler ─────────────────────────────────────────────────────

pub async fn handle_queue_info(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Response {
    let snap = state.metrics.snapshot();
    let json = serde_json::to_string(&snap.queue_depth).unwrap_or_default();
    (StatusCode::OK, [("content-type", "application/json")], json).into_response()
}

// ── Value → JSON conversion ───────────────────────────────────────────────────

fn value_to_json(v: &Value) -> JsonValue {
    match v {
        Value::Null         => JsonValue::Null,
        Value::Bool(b)      => JsonValue::Bool(*b),
        Value::Int32(n)     => JsonValue::Number((*n).into()),
        Value::Int64(n)     => JsonValue::Number((*n).into()),
        Value::Uint8(n)     => JsonValue::Number((*n).into()),
        Value::Uint16(n)    => JsonValue::Number((*n).into()),
        Value::Uint32(n)    => JsonValue::Number((*n).into()),
        Value::Uint64(n)    => serde_json::json!(*n),
        Value::Float32(f)   => serde_json::json!(*f as f64),
        Value::Float64(f)   => serde_json::json!(*f),
        Value::Vec(v)       => {
            // Try to decode as UTF-8 string first; fall back to byte array
            let bytes: Vec<u8> = v.0.iter()
                .filter_map(|b| if let Value::Uint8(u) = b { Some(*u) } else { None })
                .collect();
            if bytes.len() == v.0.len() {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    return JsonValue::String(s.to_string());
                }
            }
            JsonValue::Array(v.0.iter().map(value_to_json).collect())
        }
        Value::Struct(s)    => {
            let mut map = serde_json::Map::new();
            for (k, val) in s.0.iter() {
                map.insert(k.to_string(), value_to_json(val));
            }
            JsonValue::Object(map)
        }
        Value::Option(opt)  => match opt.as_ref() {
            fasm_vm::value::FasmOption::Some(v) => value_to_json(v),
            fasm_vm::value::FasmOption::None    => JsonValue::Null,
        },
        Value::Result(r)    => match r.as_ref() {
            fasm_vm::value::FasmResult::Ok(v)    => serde_json::json!({"ok": value_to_json(v)}),
            fasm_vm::value::FasmResult::Err(c)   => serde_json::json!({"err": c}),
        },
        _ => JsonValue::String(format!("{:?}", v)),
    }
}
