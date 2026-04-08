//! Plugin manifest: loads `*.plugin.toml` files from a plugins directory and
//! produces the metadata the sandbox needs to auto-mount sidecar processes.

use std::path::Path;

/// Parsed representation of a `*.plugin.toml` manifest file.
///
/// **Example manifest** (`plugins/db_query.plugin.toml`):
/// ```toml
/// [plugin]
/// name       = "db_query"
/// syscall_ids = [100, 101, 102]
/// cmd        = "python"
/// args       = ["plugins/db_query.py"]
/// auto_mount = true
/// ```
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Human-readable plugin name (for logging/metrics).
    pub name: String,
    /// Syscall IDs that will be routed to this sidecar process.
    pub syscall_ids: Vec<i32>,
    /// Executable to launch (e.g. `"python"`, `"./plugins/my_plugin"`).
    pub cmd: String,
    /// Additional arguments forwarded to the executable.
    pub args: Vec<String>,
    /// Whether to automatically mount this plugin at sandbox startup.
    pub auto_mount: bool,
}

/// Scan `dir` for `*.plugin.toml` files, parse each one, and return all
/// manifests where `auto_mount == true`.
///
/// Errors loading individual manifests are logged and skipped; the scan
/// never fails the whole startup.
pub fn discover_auto_mount(dir: &Path) -> Vec<PluginManifest> {
    let mut manifests = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            eprintln!(
                "[fasm-sandbox] plugin discovery: cannot read {:?}: {}",
                dir, err
            );
            return manifests;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".plugin.toml"))
                .unwrap_or(false)
        {
            match load_manifest(&path) {
                Ok(m) if m.auto_mount => manifests.push(m),
                Ok(_) => {}
                Err(e) => eprintln!("[fasm-sandbox] plugin manifest {:?}: {}", path, e),
            }
        }
    }

    manifests
}

/// Load a single `*.plugin.toml` file.
pub fn load_manifest(path: &Path) -> Result<PluginManifest, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read error: {}", e))?;
    parse_manifest(&raw)
}

/// Parse TOML text into a [`PluginManifest`].
///
/// We keep a manual parser to avoid pulling the full `toml` crate into
/// `fasm-sandbox` — the format is simple enough to handle line-by-line.
pub fn parse_manifest(toml: &str) -> Result<PluginManifest, String> {
    let mut name = None::<String>;
    let mut cmd = None::<String>;
    let mut args: Vec<String> = Vec::new();
    let mut syscall_ids: Vec<i32> = Vec::new();
    let mut auto_mount = false;
    let mut in_plugin = false;

    // Collect logical lines (handle arrays that may span multiple lines).
    let mut logical_lines: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut in_array = false;

    for raw in toml.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if in_array {
            buf.push(' ');
            buf.push_str(line);
            if line.contains(']') {
                in_array = false;
                logical_lines.push(buf.trim().to_string());
                buf.clear();
            }
        } else if line.contains('[') && !line.starts_with('[') && !line.contains(']') {
            // key = [   starts an array split across lines
            buf = line.to_string();
            in_array = true;
        } else {
            logical_lines.push(line.to_string());
        }
    }

    for line in &logical_lines {
        if line == "[plugin]" {
            in_plugin = true;
            continue;
        }
        if line.starts_with('[') {
            in_plugin = false;
            continue;
        }
        if !in_plugin {
            continue;
        }

        if let Some((k, v)) = split_kv(line) {
            match k {
                "name" => name = Some(strip_quotes(v)),
                "cmd" => cmd = Some(strip_quotes(v)),
                "auto_mount" => auto_mount = v.trim() == "true",
                "args" => args = parse_str_array(v),
                "syscall_ids" => syscall_ids = parse_int_array(v)?,
                _ => {}
            }
        }
    }

    Ok(PluginManifest {
        name: name.unwrap_or_default(),
        cmd: cmd.ok_or("missing cmd in [plugin]")?,
        args,
        syscall_ids,
        auto_mount,
    })
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    Some((line[..eq].trim(), line[eq + 1..].trim()))
}

fn strip_quotes(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

/// Parse `["a", "b", "c"]` → `vec!["a", "b", "c"]`.
fn parse_str_array(s: &str) -> Vec<String> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|t| strip_quotes(t.trim()))
        .filter(|t| !t.is_empty())
        .collect()
}

/// Parse `[100, 101, 102]` → `vec![100, 101, 102]`.
fn parse_int_array(s: &str) -> Result<Vec<i32>, String> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|t| {
            t.trim()
                .parse::<i32>()
                .map_err(|e| format!("bad syscall_id: {}", e))
        })
        .collect()
}
