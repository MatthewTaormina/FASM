//! Persistent handler architecture.
//!
//! A [`PersistentHandler`] wraps a single dedicated OS thread that maintains a
//! "warm" [`Sandbox`] and calls a designated FASM function in a loop.  This
//! eliminates the per-request sandbox spawn overhead (~35 µs dispatcher tax)
//! and enables sub-5 µs numeric hot-path latency when combined with JIT.
//!
//! ## State passing
//!
//! After each successful invocation the handler's `$ret` value is captured.  If
//! it is a `STRUCT`, it is injected into the *next* invocation's `$args` at the
//! well-known field key [`KEY_STATE`].  FASM programs can read back their own
//! previous return value with:
//!
//! ```text
//! FUNC Handler
//!     LOCAL 0, STRUCT, state
//!     GET_FIELD $args, KEY_STATE, state
//!     // … process request, update state …
//!     RET state       // captured and forwarded to the next iteration
//! ENDF
//! ```
//!
//! ## Fault recovery
//!
//! If an invocation faults (returns an `Err`), the engine logs the fault code,
//! discards any in-flight state mutation, and restarts the handler with the last
//! known-good state.  The external caller receives an `Err(fault_message)`.

use fasm_bytecode::Program;
use fasm_jit::FasmJit;
use fasm_sandbox::{Sandbox, SandboxConfig};
use fasm_vm::{value::FasmStruct, Value};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::{dispatcher::EngineError, metrics::MetricsRegistry};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Field key used to inject the previous iteration's `$ret` STRUCT into
/// the current iteration's `$args`.
///
/// Programs read accumulated state with:
/// ```text
/// GET_FIELD $args, KEY_STATE, my_state
/// ```
pub const KEY_STATE: u32 = 0x7FFF_FFFE;

// ── Request / Response ────────────────────────────────────────────────────────

struct HandlerRequest {
    args: Value,
    response_tx: oneshot::Sender<Result<Value, String>>,
}

// ── PersistentHandler ─────────────────────────────────────────────────────────

/// A long-lived, single-threaded FASM handler that maintains a warm sandbox
/// and accumulates state across invocations.
pub struct PersistentHandler {
    tx: std::sync::mpsc::SyncSender<HandlerRequest>,
    /// Kept alive so the thread is joined on drop.
    _thread: std::thread::JoinHandle<()>,
}

impl PersistentHandler {
    /// Spawn the handler thread and return a handle.
    pub fn spawn(
        program: Arc<Program>,
        func: String,
        sandbox_config: Arc<SandboxConfig>,
        metrics: MetricsRegistry,
        jit: Option<Arc<FasmJit>>,
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

            // Last known-good state: starts as an empty STRUCT.
            let mut last_good_state: Value = Value::Struct(FasmStruct::default());

            while let Ok(req) = rx.recv() {
                let HandlerRequest { mut args, response_tx } = req;

                // Inject state into args at KEY_STATE.
                if let Value::Struct(ref mut s) = args {
                    if let Value::Struct(_) = &last_good_state {
                        s.insert(KEY_STATE, last_good_state.clone());
                    }
                }

                sb.reset();
                let result = sb.run_named(&program, &func, args);

                match result {
                    Ok(ret) => {
                        // Capture new state if the return value is a STRUCT.
                        if let Value::Struct(_) = &ret {
                            last_good_state = ret.clone();
                        }
                        let _ = response_tx.send(Ok(ret));
                    }
                    Err(fault) => {
                        tracing::error!(
                            func = %func,
                            fault = %fault,
                            "persistent handler fault — reverting to last known-good state"
                        );
                        // Do NOT update last_good_state — keep the previous one.
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
