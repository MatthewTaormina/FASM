//! Engine bootstrap — wires all subsystems from a parsed config.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use axum::{Router, routing::get};
use fasm_sandbox::SandboxConfig;
use tokio::net::TcpListener;

use crate::{
    config::EngineConfig,
    dispatcher::TaskDispatcher,
    http_handler::{AppState, handle_request, handle_metrics, handle_queue_info},
    metrics::MetricsRegistry,
    queue_looper::spawn_queue_looper,
    queues::QueueRegistry,
    router::RouteTable,
    scheduler::spawn_schedule,
};

/// Shared engine state accessible to all subsystems.
#[derive(Clone)]
pub struct EngineState {
    pub config:     Arc<EngineConfig>,
    pub metrics:    MetricsRegistry,
    pub queues:     QueueRegistry,
    pub dispatcher: TaskDispatcher,
}

/// Start the engine from an `engine.toml` config.
///
/// Binds to `config.server.host:config.server.port` and runs until the server
/// closes.  Never returns on success.
pub async fn run(config: EngineConfig, config_dir: PathBuf) -> Result<(), String> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&addr).await
        .map_err(|e| format!("bind failed on {}: {}", addr, e))?;
    run_with_listener(config, config_dir, listener).await.map(|_| ())
}

/// Start the engine on an already-bound `TcpListener`.
///
/// Useful for tests: bind to port 0, obtain the OS-assigned address, then start
/// the engine.  This function returns the bound `SocketAddr` and then runs the
/// axum server until the future is dropped/cancelled.
pub async fn run_with_listener(
    config:     EngineConfig,
    config_dir: PathBuf,
    listener:   TcpListener,
) -> Result<SocketAddr, String> {
    tracing::info!("fasm-engine starting up");

    let bound_addr = listener.local_addr()
        .map_err(|e| format!("cannot read local addr: {}", e))?;

    let metrics = MetricsRegistry::new();
    let queues  = QueueRegistry::new();

    // Pre-register all declared queues.
    for qcfg in &config.queues {
        queues.get_or_create(&qcfg.name, qcfg.max_retries, qcfg.timeout_secs);
    }

    // Build sandbox config.
    let sandbox_config = Arc::new(SandboxConfig {
        clock_hz: config.engine.clock_hz,
        plugin_discovery_dir: config.plugins.discovery_dir
            .as_ref()
            .map(|d| config_dir.join(d)),
    });

    let dispatcher = TaskDispatcher::new_with_config(
        config.engine.max_concurrent,
        metrics.clone(),
        sandbox_config,
    );

    // Compile routes.
    let route_table = Arc::new(
        RouteTable::from_configs(&config.routes, &config_dir)
            .map_err(|e| format!("route compilation failed: {}", e))?
    );

    // Spawn schedule tasks.
    for sched in config.schedules.clone() {
        let d   = dispatcher.clone();
        let dir = config_dir.clone();
        match spawn_schedule(sched, &dir, d) {
            Ok(_) => {}
            Err(e) => tracing::error!("schedule spawn failed: {}", e),
        }
    }

    // Spawn queue loopers.
    for qcfg in config.queues.clone() {
        let d   = dispatcher.clone();
        let dir = config_dir.clone();
        let qr  = queues.clone();
        let m   = metrics.clone();
        match spawn_queue_looper(qcfg, &dir, &qr, d, m) {
            Ok(_) => {}
            Err(e) => tracing::error!("queue looper spawn failed: {}", e),
        }
    }

    // Build axum router / state.
    let app_state = AppState {
        routes:     route_table,
        dispatcher: dispatcher.clone(),
        metrics:    metrics.clone(),
    };

    let app = Router::new()
        .route("/metrics",      get(handle_metrics))
        .route("/admin/queues", get(handle_queue_info))
        .fallback(handle_request)
        .with_state(app_state);

    tracing::info!(addr = %bound_addr, "listening");

    axum::serve(listener, app).await
        .map_err(|e| format!("server error: {}", e))?;

    Ok(bound_addr)
}
