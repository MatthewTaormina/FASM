//! Persistent handler architecture.
//!
//! A [`PersistentHandler`] wraps a single dedicated OS thread that maintains a
//! "warm" [`Sandbox`] and calls a designated FASM function in a loop.  This
//! eliminates the per-request sandbox spawn overhead (~35 µs dispatcher tax)
//! and enables sub-5 µs numeric hot-path latency when combined with JIT.
//!
//! ## Stateless calls
//!
//! Each invocation is **fully independent** — the sandbox call stack is reset
//! between requests and no data leaks from one call to the next.  The
//! "persistent" aspect refers only to the warm sandbox (JIT compiled, syscall
//! table pre-mounted), not to any shared mutable state.
//!
//! ## Environment variable injection
//!
//! If [`EnvVarBinding`]s are configured for a handler, the engine reads the
//! corresponding OS environment variables once per invocation and injects them
//! as a `STRUCT` at the well-known field key [`KEY_ENV`] in `$args`.
//!
//! ```text
//! FUNC Handler
//!     LOCAL 0, STRUCT, env
//!     LOCAL 1, VEC,    db_url
//!     GET_FIELD $args, KEY_ENV, env
//!     GET_FIELD env, 0, db_url    // env var bound to key 0
//!     // … use db_url …
//!     RET
//! ENDF
//! ```
//!
//! Config (`engine.toml`):
//! ```toml
//! [[handlers]]
//! name     = "worker"
//! source   = "worker.fasm"
//! function = "Handler"
//!
//! [[handlers.env_bindings]]
//! key = 0
//! var = "DATABASE_URL"
//! ```

use fasm_bytecode::Program;
use fasm_jit::FasmJit;
use fasm_sandbox::{Sandbox, SandboxConfig};
use fasm_vm::{
    value::{FasmStruct, FasmVec},
    Value,
};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::{
    config::EnvVarBinding,
    dispatcher::EngineError,
    metrics::MetricsRegistry,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Field key in `$args` at which environment variable bindings are injected.
///
/// The value at this key is a `STRUCT { binding.key → VEC<UINT8> }`.
/// If a bound variable is not set in the OS environment the field is `NULL`.
///
/// ```text
/// GET_FIELD $args, KEY_ENV, env
/// GET_FIELD env, 0, db_url   // reads the var bound to key 0
/// ```
pub const KEY_ENV: u32 = 0x7FFF_FFFD;

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Build the env sub-struct from a list of config-defined bindings.
/// Returns `None` if `bindings` is empty (no field is injected).
pub(crate) fn build_env_value(bindings: &[EnvVarBinding]) -> Option<Value> {
    if bindings.is_empty() {
        return None;
    }
    let mut env = FasmStruct::default();
    for b in bindings {
        let val = Value::Vec(FasmVec(b.value.bytes().map(Value::Uint8).collect()));
        env.insert(b.key, val);
    }
    Some(Value::Struct(env))
}

/// Inject env var bindings into an existing args struct at [`KEY_ENV`].
pub(crate) fn inject_env(args: &mut Value, bindings: &[EnvVarBinding]) {
    if let Some(env_val) = build_env_value(bindings) {
        if let Value::Struct(ref mut s) = args {
            s.insert(KEY_ENV, env_val);
        }
    }
}

// ── Request / Response ────────────────────────────────────────────────────────

struct HandlerRequest {
    args: Value,
    response_tx: oneshot::Sender<Result<Value, String>>,
}

// ── PersistentHandler ─────────────────────────────────────────────────────────

/// A long-lived, single-threaded FASM handler that keeps a warm sandbox.
///
/// Each call is **stateless** — the sandbox call stack is reset between
/// requests.  Env var bindings (if any) are re-read from the OS environment
/// on every invocation so that runtime changes are picked up automatically.
pub struct PersistentHandler {
    tx: std::sync::mpsc::SyncSender<HandlerRequest>,
    /// Kept alive so the thread is joined on drop.
    _thread: std::thread::JoinHandle<()>,
}

impl PersistentHandler {
    /// Spawn the handler thread and return a handle.
    ///
    /// `env_bindings` is captured by the thread and used to inject OS env vars
    /// into `$args` at [`KEY_ENV`] before each invocation.
    pub fn spawn(
        program: Arc<Program>,
        func: String,
        sandbox_config: Arc<SandboxConfig>,
        metrics: MetricsRegistry,
        jit: Option<Arc<FasmJit>>,
        env_bindings: Vec<EnvVarBinding>,
    ) -> Self {
        // Bounded channel — if the caller is faster than the handler it will
        // see an `Overloaded` error rather than growing memory unboundedly.
        let (tx, rx) = std::sync::mpsc::sync_channel::<HandlerRequest>(256);

        let thread = std::thread::spawn(move || {
            // One warm sandbox for the lifetime of this thread.
            let mut sb = Sandbox::from_config(0, &sandbox_config);
            if let Some(jit_arc) = jit {
                sb.set_jit(jit_arc);
            }

            // Mount engine metrics syscalls (same set as the task dispatcher).
            crate::dispatcher::mount_sandbox_syscalls(&mut sb, &metrics);

            while let Ok(req) = rx.recv() {
                let HandlerRequest { mut args, response_tx } = req;

                // Inject env vars — read fresh from OS environment each call.
                inject_env(&mut args, &env_bindings);

                // Reset call stack so each invocation starts with a clean frame.
                sb.reset();

                let result = sb.run_named(&program, &func, args);

                match result {
                    Ok(ret) => {
                        let _ = response_tx.send(Ok(ret));
                    }
                    Err(fault) => {
                        tracing::error!(func = %func, fault = %fault, "persistent handler fault");
                        let _ = response_tx.send(Err(fault));
                    }
                }
            }
        });

        Self { tx, _thread: thread }
    }

    /// Dispatch a single request to the handler and await the response.
    ///
    /// # Errors
    /// - [`EngineError::Overloaded`] if the handler's request queue is full.
    /// - [`EngineError::FasmFault`] if the FASM invocation faults.
    pub async fn dispatch(&self, args: Value) -> Result<Value, EngineError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .try_send(HandlerRequest { args, response_tx })
            .map_err(|_| EngineError::Overloaded)?;
        response_rx
            .await
            .map_err(|_| EngineError::FasmFault("persistent handler thread dropped".into()))?
            .map_err(EngineError::FasmFault)
    }
}

// ── Registry ──────────────────────────────────────────────────────────────────

/// Lookup table: handler name → [`PersistentHandler`].
#[derive(Default, Clone)]
pub struct PersistentHandlerRegistry {
    inner: Arc<std::collections::HashMap<String, Arc<PersistentHandler>>>,
}

impl PersistentHandlerRegistry {
    pub fn new(map: std::collections::HashMap<String, Arc<PersistentHandler>>) -> Self {
        Self { inner: Arc::new(map) }
    }

    pub fn get(&self, name: &str) -> Option<Arc<PersistentHandler>> {
        self.inner.get(name).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
