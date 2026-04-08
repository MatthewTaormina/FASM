//! Engine bootstrap — wires all subsystems from a parsed config.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use axum::{Router, routing::get};
use fasm_sandbox::SandboxConfig;
use tokio::{net::TcpListener, sync::RwLock};

use crate::{
    admin::{AppRegistry, router as admin_router},
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
        enable_seccomp: config.engine.enable_seccomp,
        enable_landlock: config.engine.enable_landlock,
        landlock_allowed_read_paths: config.engine.landlock_allowed_read_paths.clone(),
    });

    let dispatcher = TaskDispatcher::new_with_config(
        config.engine.max_concurrent,
        metrics.clone(),
        sandbox_config,
    );

    // ── RouteTable (dynamic, RwLock-guarded) ─────────────────────────────────
    let route_table = {
        let static_routes = RouteTable::from_configs(&config.routes, &config_dir)
            .map_err(|e| format!("route compilation failed: {}", e))?;
        Arc::new(RwLock::new(static_routes))
    };

    // ── AppRegistry — load persisted apps from data_dir ───────────────────────
    let data_dir = config_dir.join(&config.storage.data_dir);
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("cannot create data_dir {:?}: {}", data_dir, e))?;

    let registry = AppRegistry::new(data_dir);
    let persisted = registry.load_from_disk().await;

    // Re-register persisted managed routes into the live RouteTable.
    {
        let mut table = route_table.write().await;
        for manifest in &persisted {
            for route_rec in &manifest.routes {
                let file_path = registry.file_path(&manifest.namespace, &manifest.app, &route_rec.file);
                match crate::router::compile_source_file(&file_path) {
                    Ok(prog) => {
                        let prog = Arc::new(prog);
                        if let Err(e) = table.add_route_dyn(
                            &route_rec.method,
                            &route_rec.path,
                            route_rec.function.clone(),
                            prog,
                        ) {
                            tracing::warn!("startup: skipping persisted route (conflict): {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("startup: cannot compile persisted route file {:?}: {}", file_path, e);
                    }
                }
            }
        }
    }

    // Spawn scheduled tasks.
    for sched in config.schedules.clone() {
        let d   = dispatcher.clone();
        let dir = config_dir.clone();
        match spawn_schedule(sched, &dir, d) {
            Ok(_)  => {}
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
            Ok(_)  => {}
            Err(e) => tracing::error!("queue looper spawn failed: {}", e),
        }
    }

    // Build axum AppState.
    let app_state = AppState {
        routes:      route_table,
        dispatcher:  dispatcher.clone(),
        metrics:     metrics.clone(),
        admin_token: config.storage.admin_token.clone(),
        registry,
    };

    let app = Router::new()
        .route("/metrics",      get(handle_metrics))
        .route("/admin/queues", get(handle_queue_info))
        .merge(admin_router())
        .fallback(handle_request)
        .with_state(app_state);

    tracing::info!(addr = %bound_addr, "listening");

    axum::serve(listener, app).await
        .map_err(|e| format!("server error: {}", e))?;

    Ok(bound_addr)
}
