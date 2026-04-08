//! Shared test helpers for fasm-engine integration tests.
//!
//! # Quick start
//! ```
//! let engine = TestEngine::start_fixtures(128).await;
//! let status = engine.get("/ping").await.status();
//! assert_eq!(status, 200);
//! engine.shutdown(); // also called on drop
//! ```

#![allow(dead_code)]

use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio::task::JoinHandle;

use fasm_engine::{
    config::{
        EngineConfig, EngineSettings, PluginsConfig, RouteConfig, ServerConfig, StorageConfig,
    },
    engine::run_with_listener,
};

/// A running engine instance for tests.
pub struct TestEngine {
    pub base_url: String,
    pub addr: SocketAddr,
    handle: JoinHandle<()>,
    client: reqwest::Client,
}

impl TestEngine {
    /// Start the engine on a free OS port with the given route config.
    ///
    /// The `fixtures_dir` must be an absolute path to the directory that
    /// contains the `.fasm` source files referenced by `routes`.
    pub async fn start(
        routes: Vec<RouteConfig>,
        fixtures_dir: PathBuf,
        max_concurrent: usize,
    ) -> Self {
        let config = EngineConfig {
            server: ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            engine: EngineSettings {
                max_concurrent,
                hot_reload: false,
                clock_hz: 0,
                enable_seccomp: false,
                enable_landlock: false,
                landlock_allowed_read_paths: vec![],
            },
            plugins: PluginsConfig {
                discovery_dir: None,
            },
            storage: StorageConfig::default(),
            routes,
            schedules: vec![],
            queues: vec![],
            events: vec![],
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind to port 0");
        let addr = listener.local_addr().expect("local addr");
        let base_url = format!("http://{}", addr);

        // Kick off the engine in a background task.
        let handle = tokio::spawn(async move {
            if let Err(e) = run_with_listener(config, fixtures_dir, listener).await {
                eprintln!("[TestEngine] engine error: {}", e);
            }
        });

        // Wait until the engine is accepting connections (poll /metrics).
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let metrics_url = format!("{}/metrics", base_url);
        for _ in 0..50 {
            if let Ok(r) = client.get(&metrics_url).send().await {
                if r.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        TestEngine {
            base_url,
            addr,
            handle,
            client,
        }
    }

    /// Convenience: start with the three standard fixtures (ping, echo, fib).
    pub async fn start_fixtures(max_concurrent: usize) -> Self {
        let fixtures_dir = fixtures_dir();
        let routes = vec![
            RouteConfig {
                method: "GET".into(),
                path: "/ping".into(),
                function: "Ping".into(),
                source: "ping.fasm".into(),
            },
            RouteConfig {
                method: "GET".into(),
                path: "/echo/:word".into(),
                function: "Echo".into(),
                source: "echo.fasm".into(),
            },
            RouteConfig {
                method: "GET".into(),
                path: "/fib".into(),
                function: "FibHandler".into(),
                source: "fib_handler.fasm".into(),
            },
        ];
        Self::start(routes, fixtures_dir, max_concurrent).await
    }

    pub async fn get(&self, path: &str) -> reqwest::Response {
        let url = format!("{}{}", self.base_url, path);
        self.client
            .get(&url)
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {} failed: {}", url, e))
    }

    pub async fn post_json(&self, path: &str, body: &serde_json::Value) -> reqwest::Response {
        let url = format!("{}{}", self.base_url, path);
        self.client
            .post(&url)
            .json(body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {} failed: {}", url, e))
    }

    pub fn shutdown(self) {
        self.handle.abort();
    }
}

impl Drop for TestEngine {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Absolute path to `tests/fixtures/`.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}
