//! Engine configuration — parsed from `engine.toml`.

use serde::Deserialize;
use std::path::PathBuf;

/// Top-level `engine.toml` schema.
#[derive(Debug, Clone, Deserialize)]
pub struct EngineConfig {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub plugins: PluginsConfig,

    #[serde(default)]
    pub engine: EngineSettings,

    #[serde(default)]
    pub routes: Vec<RouteConfig>,

    #[serde(default)]
    pub schedules: Vec<ScheduleConfig>,

    #[serde(default)]
    pub queues: Vec<QueueConfig>,

    #[serde(default)]
    pub events: Vec<EventConfig>,
}

// ── Subsections ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { host: default_host(), port: default_port() }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PluginsConfig {
    /// Directory to scan for `*.plugin.toml` manifests at startup.
    pub discovery_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EngineSettings {
    /// Maximum FASM executions allowed to run concurrently.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// When true, watch `.fasm` source files and recompile on change.
    #[serde(default)]
    pub hot_reload: bool,
    /// Clock limit per sandbox execution (0 = unlimited).
    #[serde(default)]
    pub clock_hz: u64,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            hot_reload: false,
            clock_hz: 0,
        }
    }
}

// ── Route ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    /// HTTP method, e.g. `"GET"`, `"POST"`.
    pub method: String,
    /// Path pattern, e.g. `/api/users/:id`.
    pub path: String,
    /// FASM function to invoke as the handler.
    pub function: String,
    /// Path to `.fasm` source or `.fasmc` compiled bytecode.
    pub source: String,
}

// ── Schedule ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleConfig {
    pub name: String,
    /// Cron expression, e.g. `"0 */5 * * * *"`.
    pub cron: String,
    /// FASM function to invoke on each tick.
    pub function: String,
    pub source: String,
    /// What to do if a previous tick is still running.
    #[serde(default = "default_misfire")]
    pub misfire_policy: MisfirePolicy,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    /// Skip the fired tick (default).
    Skip,
    /// Run the tick even if a previous one is still in flight.
    RunAll,
}

impl Default for MisfirePolicy {
    fn default() -> Self { MisfirePolicy::Skip }
}

// ── Queue ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct QueueConfig {
    pub name: String,
    #[serde(default = "default_queue_type")]
    pub r#type: QueueType,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// FASM function invoked for each dequeued message (for shared queues).
    pub function: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QueueType {
    #[default]
    Shared,
    FunctionScoped,
}

// ── Event ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct EventConfig {
    pub name: String,
    pub function: String,
    pub source: String,
}

// ── defaults ─────────────────────────────────────────────────────────────────

fn default_host()           -> String { "0.0.0.0".to_string() }
fn default_port()           -> u16    { 8080 }
fn default_max_concurrent() -> usize  { 256 }
fn default_misfire()        -> MisfirePolicy { MisfirePolicy::Skip }
fn default_queue_type()     -> QueueType { QueueType::Shared }
fn default_max_retries()    -> u32 { 3 }
fn default_timeout_secs()   -> u64 { 30 }

// ── loader ────────────────────────────────────────────────────────────────────

/// Read and parse `engine.toml` from `path`.
pub fn load(path: &std::path::Path) -> Result<EngineConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read config {:?}: {}", path, e))?;
    toml::from_str(&raw)
        .map_err(|e| format!("config parse error: {}", e))
}
