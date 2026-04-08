//! Queue looper — one perpetual async task per named shared queue.
//!
//! Each looper:
//! 1. Polls its queue for ready messages in a tight async loop (with a small
//!    sleep to avoid busy-waiting when the queue is empty).
//! 2. For each message: calls `dispatcher.spawn_fire_and_forget()` so the
//!    looper is **never blocked** by in-flight FASM executions.
//! 3. Periodically calls `queue.requeue_expired()` to return timed-out messages.
//!
//! If the semaphore is exhausted (Dropped), the message stays in the queue
//! for the next poll cycle — nothing is lost.

use fasm_bytecode::Program;
use fasm_compiler::compile_source;
use fasm_vm::value::{FasmStruct, FasmVec};
use fasm_vm::Value;
use std::{path::Path, sync::Arc, time::Duration};

use crate::{
    config::QueueConfig,
    dispatcher::{EngineError, ExecRequest, TaskDispatcher},
    metrics::MetricsRegistry,
    queues::{QueueRegistry, SharedQueue},
};

/// Spawn one queue-looper task.
pub fn spawn_queue_looper(
    cfg: QueueConfig,
    base_dir: &Path,
    queue_registry: &QueueRegistry,
    dispatcher: TaskDispatcher,
    metrics: MetricsRegistry,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let func = match &cfg.function {
        Some(f) => f.clone(),
        None => return Ok(tokio::spawn(async {})),
    };
    let source = match &cfg.source {
        Some(s) => s.clone(),
        None => return Ok(tokio::spawn(async {})),
    };

    let source_path = base_dir.join(&source);
    let src = std::fs::read_to_string(&source_path)
        .map_err(|e| format!("queue looper: cannot read {:?}: {}", source_path, e))?;
    let program: Arc<Program> =
        Arc::new(compile_source(&src).map_err(|e| format!("queue looper compile error: {}", e))?);

    let queue = queue_registry.get_or_create(&cfg.name, cfg.max_retries, cfg.timeout_secs);
    let name = cfg.name.clone();

    let handle = tokio::spawn(async move {
        tracing::info!(queue = %name, func = %func, "queue looper started");
        let mut expire_counter: u32 = 0;

        loop {
            // Re-queue expired in-flight messages every ~30 iterations.
            expire_counter += 1;
            if expire_counter >= 30 {
                queue.requeue_expired();
                expire_counter = 0;
            }

            // Update queue depth metric.
            metrics.set_queue_depth(&name, queue.depth() as u64);

            match queue.try_dequeue() {
                Some(msg) => {
                    // Build $args: field 0 = message payload as JSON string bytes,
                    //              field 1 = message id bytes.
                    let payload_str = msg.payload.to_string();
                    let payload_bytes: Vec<Value> = payload_str.bytes().map(Value::Uint8).collect();
                    let id_bytes: Vec<Value> = msg.id.bytes().map(Value::Uint8).collect();

                    let mut args_struct = FasmStruct::default();
                    args_struct.insert(0u32, Value::Vec(FasmVec(payload_bytes)));
                    args_struct.insert(1u32, Value::Vec(FasmVec(id_bytes)));

                    let req = ExecRequest {
                        func: func.clone(),
                        program: program.clone(),
                        args: Value::Struct(args_struct),
                        trigger: "queue".to_string(),
                    };

                    match dispatcher.spawn_fire_and_forget(req) {
                        Ok(_) => {}
                        Err(EngineError::Overloaded) => {
                            // Message is already in-flight tracking; it will expire and re-queue.
                            tracing::warn!(queue = %name, msg_id = %msg.id, "looper: engine overloaded, message will be retried on ack timeout");
                        }
                        Err(e) => tracing::error!(queue = %name, "looper error: {}", e),
                    }
                }
                None => {
                    // No messages ready; sleep a bit to avoid busy-looping.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    });

    Ok(handle)
}
