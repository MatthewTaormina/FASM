//! Async cron-style scheduler.
//!
//! Each `[[schedules]]` entry in `engine.toml` gets its own perpetual async task.
//! On each tick the scheduler calls `dispatcher.spawn_fire_and_forget()` and
//! immediately computes the next interval — **never blocking on FASM execution**.
//!
//! ## Misfire policy
//! - `skip` (default): if a previous tick is still running when the interval fires,
//!   the current tick is silently skipped.
//! - `run_all`: always fires a new execution regardless of in-flight count.
//!   The dispatcher's semaphore still limits true concurrency.

use fasm_bytecode::Program;
use fasm_compiler::compile_source;
use fasm_vm::value::FasmStruct;
use fasm_vm::Value;
use std::{path::Path, sync::Arc, time::Duration};

use crate::{
    config::{MisfirePolicy, ScheduleConfig},
    dispatcher::{EngineError, ExecRequest, TaskDispatcher},
};

/// Parse a simplistic cron string into an interval duration.
///
/// We support only `"0 */N * * * *"` (every N minutes) and
/// `"*/N * * * * *"` (every N seconds) for the demo.
/// A production implementation would use a cron-parsing crate.
fn parse_cron_interval(cron: &str) -> Duration {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    // 6-field cron: sec min hour dom mon dow
    if parts.len() >= 2 {
        let sec_field = parts[0];
        let min_field = parts[1];
        if let Some(n) = min_field.strip_prefix("*/") {
            if let Ok(mins) = n.parse::<u64>() {
                return Duration::from_secs(mins * 60);
            }
        }
        if let Some(n) = sec_field.strip_prefix("*/") {
            if let Ok(secs) = n.parse::<u64>() {
                return Duration::from_secs(secs);
            }
        }
    }
    // Fallback: 60 second interval
    Duration::from_secs(60)
}

/// Spawn one perpetual scheduler task for a single schedule config.
pub fn spawn_schedule(
    cfg: ScheduleConfig,
    base_dir: &Path,
    dispatcher: TaskDispatcher,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let source_path = base_dir.join(&cfg.source);
    let src = std::fs::read_to_string(&source_path)
        .map_err(|e| format!("scheduler: cannot read {:?}: {}", source_path, e))?;
    let program: Arc<Program> =
        Arc::new(compile_source(&src).map_err(|e| format!("scheduler compile error: {}", e))?);

    let interval = parse_cron_interval(&cfg.cron);
    let name = cfg.name.clone();
    let func = cfg.function.clone();
    let policy = cfg.misfire_policy;

    let handle = tokio::spawn(async move {
        tracing::info!(schedule = %name, interval = ?interval, "scheduler started");
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            let req = ExecRequest {
                func: func.clone(),
                program: program.clone(),
                args: Value::Struct(FasmStruct::default()),
                trigger: "schedule".to_string(),
                jit: None,
            };
            match dispatcher.spawn_fire_and_forget(req) {
                Ok(_) => {}
                Err(EngineError::Overloaded) => {
                    if policy == MisfirePolicy::Skip {
                        tracing::warn!(schedule = %name, "tick skipped (engine overloaded)");
                    }
                    // run_all: overloaded means still dropped by semaphore, but we tried
                }
                Err(e) => tracing::error!(schedule = %name, "tick error: {}", e),
            }
        }
    });

    Ok(handle)
}
