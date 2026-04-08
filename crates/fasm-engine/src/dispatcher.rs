//! TaskDispatcher — the non-blocking execution primitive.
//!
//! Every FASM invocation flows through here.  The dispatcher:
//! 1. Acquires a `Semaphore` permit (bounded concurrency).
//! 2. Moves the VM into a `tokio::task::spawn_blocking` call (CPU-bound sync work).
//! 3. Either awaits the result (`spawn_async`) or fires-and-forgets (`spawn_fire_and_forget`).
//! 4. Records metrics on completion.

use fasm_bytecode::Program;
use fasm_sandbox::{Sandbox, SandboxConfig};
use fasm_vm::Value;
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
            let mut sb = Sandbox::from_config(new_id(), &sandbox_cfg);

            // Mount engine syscalls
            mount_engine_syscalls(&mut sb, &metrics);

            let res = sb.run_named(&req.program, &req.func, req.args.clone());
            let ms = start.elapsed().as_millis() as u64;
            metrics.record_duration_ms(&req.func, ms);
            if res.is_err() {
                metrics.record_error(&req.func);
            }
            metrics.dec_active();
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
                let mut sb = Sandbox::from_config(new_id(), &sandbox_cfg2);
                mount_engine_syscalls(&mut sb, &metrics2);
                let r = sb.run_named(&req.program, &req.func, req.args.clone());
                let ms = start.elapsed().as_millis() as u64;
                metrics2.record_duration_ms(&req.func, ms);
                if r.is_err() {
                    metrics2.record_error(&req.func);
                }
                metrics2.dec_active();
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
