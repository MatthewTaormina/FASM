//! AppRegistry — persisted namespace/app/file metadata.
//!
//! Namespaces use dot-notation (e.g. `com.acme.payments`).
//! On-disk layout:
//!
//! ```text
//! data_dir/
//!   com.acme.payments/
//!     checkout/
//!       manifest.json
//!       files/
//!         handler.fasm
//!         helpers.fasmc
//! ```

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    pub namespace: String,
    pub app:       String,
    #[serde(default = "Utc::now")]
    pub created:   DateTime<Utc>,
    #[serde(default)]
    pub files:     Vec<FileRecord>,
    #[serde(default)]
    pub routes:    Vec<RouteRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub name:     String,
    pub kind:     FileKind,
    pub size:     u64,
    pub uploaded: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Source,
    Bytecode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRecord {
    pub id:       Uuid,
    pub method:   String,
    pub path:     String,
    pub function: String,
    pub file:     String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamespaceInfo {
    pub name:  String,
    pub apps:  Vec<String>,
}

// ── AppRegistry ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppRegistry {
    inner:    Arc<RwLock<RegistryInner>>,
    data_dir: PathBuf,
}

struct RegistryInner {
    /// namespace → app_name → manifest
    namespaces: HashMap<String, HashMap<String, AppManifest>>,
}

impl AppRegistry {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            inner:    Arc::new(RwLock::new(RegistryInner { namespaces: HashMap::new() })),
            data_dir,
        }
    }

    /// Load all persisted manifests from disk at engine startup.
    pub async fn load_from_disk(&self) -> Vec<AppManifest> {
        let mut out = Vec::new();
        let Ok(ns_iter) = std::fs::read_dir(&self.data_dir) else { return out };
        let mut inner = self.inner.write().await;

        for ns_entry in ns_iter.flatten() {
            if !ns_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue; }
            let ns_name = ns_entry.file_name().to_string_lossy().to_string();
            if !is_valid_namespace(&ns_name) { continue; }

            let Ok(app_iter) = std::fs::read_dir(ns_entry.path()) else { continue };
            for app_entry in app_iter.flatten() {
                if !app_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue; }
                let manifest_path = app_entry.path().join("manifest.json");
                if let Ok(raw) = std::fs::read_to_string(&manifest_path) {
                    if let Ok(manifest) = serde_json::from_str::<AppManifest>(&raw) {
                        inner.namespaces
                            .entry(ns_name.clone())
                            .or_default()
                            .insert(manifest.app.clone(), manifest.clone());
                        out.push(manifest);
                    }
                }
            }
        }
        out
    }

    // ── namespace ops ─────────────────────────────────────────────────────────

    pub async fn list_namespaces(&self) -> Vec<NamespaceInfo> {
        let inner = self.inner.read().await;
        inner.namespaces.iter().map(|(ns, apps)| NamespaceInfo {
            name: ns.clone(),
            apps: apps.keys().cloned().collect(),
        }).collect()
    }

    pub async fn create_namespace(&self, name: &str) -> Result<(), String> {
        validate_namespace(name)?;
        let ns_dir = self.data_dir.join(name);
        std::fs::create_dir_all(&ns_dir)
            .map_err(|e| format!("failed to create namespace dir: {}", e))?;
        let mut inner = self.inner.write().await;
        if inner.namespaces.contains_key(name) {
            return Err(format!("namespace '{}' already exists", name));
        }
        inner.namespaces.insert(name.to_string(), HashMap::new());
        Ok(())
    }

    pub async fn get_namespace(&self, name: &str) -> Option<NamespaceInfo> {
        let inner = self.inner.read().await;
        inner.namespaces.get(name).map(|apps| NamespaceInfo {
            name: name.to_string(),
            apps: apps.keys().cloned().collect(),
        })
    }

    pub async fn delete_namespace(&self, name: &str) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        let apps = inner.namespaces.get(name)
            .ok_or_else(|| format!("namespace '{}' not found", name))?;
        if !apps.is_empty() {
            return Err(format!("namespace '{}' is not empty ({} apps)", name, apps.len()));
        }
        inner.namespaces.remove(name);
        let ns_dir = self.data_dir.join(name);
        let _ = std::fs::remove_dir_all(&ns_dir);
        Ok(())
    }

    // ── app ops ───────────────────────────────────────────────────────────────

    pub async fn list_apps(&self, ns: &str) -> Result<Vec<String>, String> {
        let inner = self.inner.read().await;
        let apps = inner.namespaces.get(ns)
            .ok_or_else(|| format!("namespace '{}' not found", ns))?;
        Ok(apps.keys().cloned().collect())
    }

    pub async fn create_app(&self, ns: &str, app: &str) -> Result<AppManifest, String> {
        validate_name(app)?;
        let mut inner = self.inner.write().await;
        let apps = inner.namespaces.get_mut(ns)
            .ok_or_else(|| format!("namespace '{}' not found", ns))?;
        if apps.contains_key(app) {
            return Err(format!("app '{}' already exists in namespace '{}'", app, ns));
        }
        let manifest = AppManifest {
            namespace: ns.to_string(),
            app:       app.to_string(),
            created:   Utc::now(),
            files:     Vec::new(),
            routes:    Vec::new(),
        };
        // Create on-disk directories
        let files_dir = self.data_dir.join(ns).join(app).join("files");
        std::fs::create_dir_all(&files_dir)
            .map_err(|e| format!("failed to create app dirs: {}", e))?;
        persist_manifest(&self.data_dir, &manifest)?;
        apps.insert(app.to_string(), manifest.clone());
        Ok(manifest)
    }

    pub async fn get_app(&self, ns: &str, app: &str) -> Option<AppManifest> {
        let inner = self.inner.read().await;
        inner.namespaces.get(ns)?.get(app).cloned()
    }

    pub async fn delete_app(&self, ns: &str, app: &str) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        let apps = inner.namespaces.get_mut(ns)
            .ok_or_else(|| format!("namespace '{}' not found", ns))?;
        apps.remove(app)
            .ok_or_else(|| format!("app '{}' not found in namespace '{}'", app, ns))?;
        let app_dir = self.data_dir.join(ns).join(app);
        let _ = std::fs::remove_dir_all(&app_dir);
        Ok(())
    }

    // ── file ops ──────────────────────────────────────────────────────────────

    /// Store raw file bytes, detect source vs bytecode, return updated manifest.
    pub async fn store_file(
        &self,
        ns:       &str,
        app:      &str,
        filename: &str,
        data:     &[u8],
    ) -> Result<FileRecord, String> {
        validate_filename(filename)?;
        let kind = if data.starts_with(b"FSMC") { FileKind::Bytecode } else { FileKind::Source };
        let file_path = self.data_dir.join(ns).join(app).join("files").join(filename);
        std::fs::write(&file_path, data)
            .map_err(|e| format!("failed to write file: {}", e))?;

        let record = FileRecord {
            name:     filename.to_string(),
            kind,
            size:     data.len() as u64,
            uploaded: Utc::now(),
        };

        let mut inner = self.inner.write().await;
        let manifest = inner.namespaces.get_mut(ns)
            .and_then(|a| a.get_mut(app))
            .ok_or_else(|| format!("app '{}/{}' not found", ns, app))?;
        // Replace or append.
        if let Some(pos) = manifest.files.iter().position(|f| f.name == filename) {
            manifest.files[pos] = record.clone();
        } else {
            manifest.files.push(record.clone());
        }
        persist_manifest(&self.data_dir, manifest)?;
        Ok(record)
    }

    pub async fn get_file_path(&self, ns: &str, app: &str, filename: &str) -> Option<PathBuf> {
        let path = self.data_dir.join(ns).join(app).join("files").join(filename);
        if path.exists() { Some(path) } else { None }
    }

    pub async fn delete_file(&self, ns: &str, app: &str, filename: &str) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        let manifest = inner.namespaces.get_mut(ns)
            .and_then(|a| a.get_mut(app))
            .ok_or_else(|| format!("app '{}/{}' not found", ns, app))?;

        // Refuse if any route still references the file.
        if manifest.routes.iter().any(|r| r.file == filename) {
            return Err(format!("file '{}' is still referenced by a route", filename));
        }

        manifest.files.retain(|f| f.name != filename);
        persist_manifest(&self.data_dir, manifest)?;
        let file_path = self.data_dir.join(ns).join(app).join("files").join(filename);
        let _ = std::fs::remove_file(&file_path);
        Ok(())
    }

    // ── route record ops ──────────────────────────────────────────────────────

    pub async fn add_route_record(
        &self,
        ns:     &str,
        app:    &str,
        record: RouteRecord,
    ) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        let manifest = inner.namespaces.get_mut(ns)
            .and_then(|a| a.get_mut(app))
            .ok_or_else(|| format!("app '{}/{}' not found", ns, app))?;
        manifest.routes.push(record);
        persist_manifest(&self.data_dir, manifest)
    }

    pub async fn remove_route_record(&self, ns: &str, app: &str, route_id: Uuid) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        let manifest = inner.namespaces.get_mut(ns)
            .and_then(|a| a.get_mut(app))
            .ok_or_else(|| format!("app '{}/{}' not found", ns, app))?;
        manifest.routes.retain(|r| r.id != route_id);
        persist_manifest(&self.data_dir, manifest)
    }

    /// Resolve absolute file path for a file in an app.
    pub fn file_path(&self, ns: &str, app: &str, filename: &str) -> PathBuf {
        self.data_dir.join(ns).join(app).join("files").join(filename)
    }
}

// ── disk persistence ──────────────────────────────────────────────────────────

fn persist_manifest(data_dir: &Path, manifest: &AppManifest) -> Result<(), String> {
    let path = data_dir
        .join(&manifest.namespace)
        .join(&manifest.app)
        .join("manifest.json");
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("serialize manifest: {}", e))?;
    // Atomic write: write to .tmp then rename.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)
        .map_err(|e| format!("write manifest tmp: {}", e))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("rename manifest: {}", e))?;
    Ok(())
}

// ── validation ────────────────────────────────────────────────────────────────

pub fn validate_namespace(name: &str) -> Result<(), String> {
    if name.is_empty() { return Err("namespace name is empty".into()); }
    for ch in name.chars() {
        if !ch.is_alphanumeric() && ch != '.' && ch != '-' && ch != '_' {
            return Err(format!("namespace '{}': invalid character '{}'", name, ch));
        }
    }
    if name.contains("..") { return Err("namespace may not contain '..'".into()); }
    Ok(())
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() { return Err("name is empty".into()); }
    for ch in name.chars() {
        if !ch.is_alphanumeric() && ch != '-' && ch != '_' {
            return Err(format!("name '{}': invalid character '{}'", name, ch));
        }
    }
    Ok(())
}

pub fn validate_filename(name: &str) -> Result<(), String> {
    if name.is_empty() { return Err("filename is empty".into()); }
    // Must end in .fasm or .fasmc, no path separators
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("filename must not contain path separators".into());
    }
    if !name.ends_with(".fasm") && !name.ends_with(".fasmc") {
        return Err("filename must end in .fasm or .fasmc".into());
    }
    Ok(())
}

fn is_valid_namespace(name: &str) -> bool {
    validate_namespace(name).is_ok()
}
