//! TaskDispatcher — the non-blocking execution primitive.
//!
//! Every FASM invocation flows through here.  The dispatcher:
//! 1. Acquires a `Semaphore` permit (bounded concurrency).
//! 2. Moves the VM into a `tokio::task::spawn_blocking` call (CPU-bound sync work).
//! 3. Either awaits the result (`spawn_async`) or fires-and-forgets (`spawn_fire_and_forget`).
//! 4. Records metrics on completion.
//!
//! ## Sandbox pooling
//!
//! Creating a fresh `Sandbox` per request requires allocating a `HashMap` for the
//! syscall table and registering ~7 closures.  Instead, each blocking OS thread
//! caches its sandbox in a `thread_local!` cell.  On the first request for a
//! given thread the sandbox is created and the engine syscalls are mounted once;
//! subsequent requests on the same thread skip that setup entirely.
//!
//! Between invocations the sandbox is reset via [`Sandbox::reset`], which clears
//! the call stack.  Global FASM slots are re-initialised by `run_named` at the
//! start of every invocation, so no additional cleanup is required.

use fasm_bytecode::Program;
use fasm_jit::FasmJit;
use fasm_sandbox::{Sandbox, SandboxConfig};
use fasm_vm::Value;
use std::cell::RefCell;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::metrics::MetricsRegistry;

// ── ExecRequest ────────────────────────────────────────────────────────────────

/// Everything the dispatcher needs to launch a FASM execution.
#[derive(Clone)]
pub struct ExecRequest {
    /// Name of the FASM function to invoke.
    pub func: String,
    /// Compiled program containing the function.
    pub program: Arc<Program>,
    /// Initial `$args` struct passed to the function.
    pub args: Value,
    /// Friendly trigger label for metrics/logs (e.g. `"http"`, `"schedule"`, `"queue"`).
    pub trigger: String,
    /// Optional pre-compiled JIT cache for this program.  When set, eligible
    /// function calls bypass the bytecode interpreter.
    pub jit: Option<Arc<FasmJit>>,
}

// ── EngineError ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum EngineError {
    /// Semaphore exhausted — caller should return 503 / drop the message.
    Overloaded,
    /// FASM execution failed.
    FasmFault(String),
    /// tokio JoinError (panic in blocking thread).
    JoinError(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Overloaded => write!(f, "engine overloaded"),
            EngineError::FasmFault(e) => write!(f, "fasm fault: {}", e),
            EngineError::JoinError(e) => write!(f, "join error: {}", e),
        }
    }
}

// ── TaskDispatcher ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TaskDispatcher {
    semaphore: Arc<Semaphore>,
    metrics: MetricsRegistry,
    sandbox_config: Arc<SandboxConfig>,
}

impl TaskDispatcher {
    pub fn new(max_concurrent: usize, metrics: MetricsRegistry) -> Self {
        Self::new_with_config(max_concurrent, metrics, Arc::new(SandboxConfig::default()))
    }

    pub fn new_with_config(
        max_concurrent: usize,
        metrics: MetricsRegistry,
        sandbox_config: Arc<SandboxConfig>,
    ) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            metrics,
            sandbox_config,
        }
    }

    /// Return the shared sandbox config (used by schedulers and queue loopers).
    pub fn sandbox_config(&self) -> Arc<SandboxConfig> {
        self.sandbox_config.clone()
    }

    // ── HTTP path: await result ───────────────────────────────────────────────

    /// Spawn a FASM execution and **async-await** the result.
    ///
    /// The caller (an axum handler) `.await`s this future.  The tokio reactor
    /// is never blocked; other requests run concurrently on the async executor.
    ///
    /// Returns `Err(EngineError::Overloaded)` immediately if the semaphore is
    /// exhausted, without queuing.
    pub async fn spawn_async(&self, req: ExecRequest) -> Result<Value, EngineError> {
        let permit = self.semaphore.clone().try_acquire_owned().map_err(|_| {
            self.metrics.record_dropped(&req.trigger);
            EngineError::Overloaded
        })?;

        let func = req.func.clone();
        let _trigger = req.trigger.clone();
        let metrics = self.metrics.clone();

        metrics.record_invocation(&func);
        metrics.inc_active();

        let sandbox_cfg = self.sandbox_config.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit; // released on drop
            let start = std::time::Instant::now();

            // Reuse a pooled sandbox from this OS thread when available.
            // On the first request for a given thread the sandbox is created
            // and the engine syscalls are mounted once; subsequent requests
            // skip that setup entirely.
            let mut sb = acquire_sandbox(&sandbox_cfg, &metrics);

            if let Some(ref jit) = req.jit {
                sb.set_jit(Arc::clone(jit) as Arc<dyn fasm_vm::JitDispatcher>);
            }
            let res = sb.run_named(&req.program, &req.func, req.args.clone());
            let ms = start.elapsed().as_millis() as u64;
            metrics.record_duration_ms(&req.func, ms);
            if res.is_err() {
                metrics.record_error(&req.func);
            }
            metrics.dec_active();

            // Return the sandbox to the thread-local pool for reuse.
            release_sandbox(sb);
            res
        })
        .await
        .map_err(|e| EngineError::JoinError(e.to_string()))?
        .map_err(EngineError::FasmFault)?;

        Ok(result)
    }

    // ── Event / scheduler / queue path: fire-and-forget ──────────────────────

    /// Spawn a FASM execution and return **immediately** — the caller is never
    /// blocked by the execution.
    ///
    /// Returns `Err(EngineError::Overloaded)` if the semaphore is exhausted;
    /// the caller should handle the misfire policy (skip / re-queue).
    pub fn spawn_fire_and_forget(&self, req: ExecRequest) -> Result<(), EngineError> {
        let permit = self.semaphore.clone().try_acquire_owned().map_err(|_| {
            self.metrics.record_dropped(&req.trigger);
            EngineError::Overloaded
        })?;

        let metrics = self.metrics.clone();
        let sandbox_cfg = self.sandbox_config.clone();
        metrics.record_invocation(&req.func);
        metrics.inc_active();

        tokio::spawn(async move {
            let _permit = permit; // released on drop
            let func = req.func.clone();
            let metrics2 = metrics.clone();

            let sandbox_cfg2 = sandbox_cfg.clone();
            let res = tokio::task::spawn_blocking(move || {
                let start = std::time::Instant::now();
                let mut sb = acquire_sandbox(&sandbox_cfg2, &metrics2);
                if let Some(ref jit) = req.jit {
                    sb.set_jit(Arc::clone(jit) as Arc<dyn fasm_vm::JitDispatcher>);
                }
                let r = sb.run_named(&req.program, &req.func, req.args.clone());
                let ms = start.elapsed().as_millis() as u64;
                metrics2.record_duration_ms(&req.func, ms);
                if r.is_err() {
                    metrics2.record_error(&req.func);
                }
                metrics2.dec_active();
                release_sandbox(sb);
                r
            })
            .await;

            if let Err(e) = res {
                tracing::error!(func = %func, "spawn_blocking panic: {}", e);
            }
        });

        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn new_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ── Thread-local sandbox pool ─────────────────────────────────────────────────
//
// Each tokio blocking thread caches one `Sandbox`.  On the first request for
// a given thread the sandbox is built and the engine syscalls are mounted once.
// On every subsequent request the sandbox is taken from the cell, reset (call
// stack cleared), used, then returned to the cell — avoiding all HashMap
// allocations and closure registrations on the hot path.
//
// Safety: `thread_local!` values are per-thread by construction; no
// synchronisation is required.

thread_local! {
    static THREAD_SANDBOX: RefCell<Option<Sandbox>> = const { RefCell::new(None) };
}

/// Retrieve a sandbox ready for use: either a recycled one from the thread-local
/// cache or a freshly constructed one with all engine syscalls pre-mounted.
fn acquire_sandbox(sandbox_cfg: &Arc<SandboxConfig>, metrics: &MetricsRegistry) -> Sandbox {
    let cached = THREAD_SANDBOX.with(|cell| cell.borrow_mut().take());
    match cached {
        Some(mut sb) => {
            // Ensure clean call-stack state from any previous invocation.
            sb.reset();
            sb
        }
        None => {
            // First request on this thread — build and wire the sandbox once.
            let mut sb = Sandbox::from_config(new_id(), sandbox_cfg);
            mount_engine_syscalls(&mut sb, metrics);
            sb
        }
    }
}

/// Return a sandbox to the thread-local cache after an invocation completes.
fn release_sandbox(sb: Sandbox) {
    THREAD_SANDBOX.with(|cell| {
        *cell.borrow_mut() = Some(sb);
    });
}

/// Mount the engine-reserved syscalls (IDs 10–49) into a fresh sandbox.
///
/// These syscalls are injected by fasm-engine; fasm-vm has no knowledge of them.
fn mount_engine_syscalls(sb: &mut Sandbox, metrics: &MetricsRegistry) {
    let m = metrics.clone();

    // 20 = METRICS_INC: struct {0: key_str, 1: delta_int}
    sb.mount_syscall(
        20,
        Box::new(move |args, _globals| {
            if let fasm_vm::Value::Struct(s) = &args {
                let key = s
                    .get(&0u32)
                    .and_then(|v| {
                        if let fasm_vm::Value::Vec(vec) = v {
                            Some(
                                String::from_utf8_lossy(
                                    &vec.0
                                        .iter()
                                        .filter_map(|b| {
                                            if let fasm_vm::Value::Uint8(u) = b {
                                                Some(*u)
                                            } else {
                                                None
                                            }
                                        })
                                        .collect::<Vec<_>>(),
                                )
                                .to_string(),
                            )
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                let delta = s
                    .get(&1u32)
                    .and_then(|v| {
                        if let fasm_vm::Value::Int32(n) = v {
                            Some(*n as i64)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(1);
                m.custom_inc(&key, delta);
            }
            Ok(fasm_vm::Value::Null)
        }),
    );

    let m2 = metrics.clone();
    // 21 = METRICS_SET
    sb.mount_syscall(
        21,
        Box::new(move |args, _globals| {
            if let fasm_vm::Value::Struct(s) = &args {
                let key = s
                    .get(&0u32)
                    .and_then(|v| {
                        if let fasm_vm::Value::Vec(vec) = v {
                            Some(
                                String::from_utf8_lossy(
                                    &vec.0
                                        .iter()
                                        .filter_map(|b| {
                                            if let fasm_vm::Value::Uint8(u) = b {
                                                Some(*u)
                                            } else {
                                                None
                                            }
                                        })
                                        .collect::<Vec<_>>(),
                                )
                                .to_string(),
                            )
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                let val = s
                    .get(&1u32)
                    .and_then(|v| {
                        if let fasm_vm::Value::Int32(n) = v {
                            Some(*n as i64)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                m2.custom_set(&key, val);
            }
            Ok(fasm_vm::Value::Null)
        }),
    );

    let m3 = metrics.clone();
    // 22 = METRICS_GET
    sb.mount_syscall(
        22,
        Box::new(move |args, _globals| {
            let key = if let fasm_vm::Value::Vec(ref vec) = args {
                String::from_utf8_lossy(
                    &vec.0
                        .iter()
                        .filter_map(|b| {
                            if let fasm_vm::Value::Uint8(u) = b {
                                Some(*u)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>(),
                )
                .to_string()
            } else {
                String::new()
            };
            match m3.custom_get(&key) {
                Some(v) => Ok(fasm_vm::Value::Option(Box::new(
                    fasm_vm::value::FasmOption::Some(fasm_vm::Value::Int32(v as i32)),
                ))),
                None => Ok(fasm_vm::Value::Option(Box::new(
                    fasm_vm::value::FasmOption::None,
                ))),
            }
        }),
    );
}
