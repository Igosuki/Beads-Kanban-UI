//! Backend detection and registry for beads storage.
//!
//! Inspects a project directory to determine which storage backend should be
//! used (JSONL files or Dolt), and provides a [`BackendRegistry`] that lazily
//! creates and caches backend instances per project path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::backend::{BeadsBackend, BeadsError};
use crate::backend::dolt::DoltBackend;
use crate::backend::dolt_server::DoltServerManager;
use crate::backend::jsonl::JsonlBackend;

/// The type of storage backend detected for a project.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendType {
    /// JSONL flat-file storage (default).
    Jsonl,
    /// Dolt relational database storage.
    Dolt,
}

/// Detects which backend a project uses by inspecting its `.beads/` directory.
///
/// Detection order:
/// 1. If `.beads/config.yaml` exists and contains `no-db: true` → [`BackendType::Jsonl`]
/// 2. If `.beads/dolt/` directory exists → [`BackendType::Dolt`]
/// 3. Fallback → [`BackendType::Jsonl`]
pub fn detect_backend(project_path: &Path) -> BackendType {
    let config_path = project_path.join(".beads").join("config.yaml");

    // Check config.yaml for `no-db: true`
    if let Ok(contents) = std::fs::read_to_string(&config_path) {
        if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&contents) {
            if yaml.get("no-db").and_then(|v| v.as_bool()) == Some(true) {
                return BackendType::Jsonl;
            }
        }
    }

    // Check for Dolt directory
    let dolt_dir = project_path.join(".beads").join("dolt");
    if dolt_dir.exists() && dolt_dir.is_dir() {
        return BackendType::Dolt;
    }

    // Fallback
    BackendType::Jsonl
}

/// Registry that lazily creates and caches backend instances per project path.
///
/// The registry detects the appropriate backend type on first access and
/// returns a shared [`Arc<dyn BeadsBackend>`] for subsequent calls with the
/// same path.
pub struct BackendRegistry {
    backends: HashMap<PathBuf, Arc<dyn BeadsBackend>>,
    dolt_server_manager: Arc<Mutex<DoltServerManager>>,
}

impl BackendRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
            dolt_server_manager: Arc::new(Mutex::new(DoltServerManager::new())),
        }
    }

    /// Returns (or creates) the backend for the given project path.
    ///
    /// On the first call for a path the backend type is detected, the instance
    /// is created and cached.  Subsequent calls return the cached [`Arc`].
    pub fn get_or_create(
        &mut self,
        project_path: &Path,
    ) -> Result<Arc<dyn BeadsBackend>, BeadsError> {
        // Normalise to a canonical owned PathBuf key.
        let key = project_path.to_path_buf();

        if let Some(backend) = self.backends.get(&key) {
            return Ok(Arc::clone(backend));
        }

        let backend: Arc<dyn BeadsBackend> = match detect_backend(project_path) {
            BackendType::Jsonl => Arc::new(JsonlBackend),
            BackendType::Dolt => Arc::new(DoltBackend::new(Arc::clone(&self.dolt_server_manager))),
        };

        self.backends.insert(key, Arc::clone(&backend));
        Ok(backend)
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A thread-safe, shared handle to a [`BackendRegistry`].
///
/// Use this type as Axum application state so that route handlers can resolve
/// the correct backend for each incoming request.
#[allow(dead_code)]
pub type SharedBackendRegistry = Arc<RwLock<BackendRegistry>>;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_beads_dir(tmp: &TempDir) -> PathBuf {
        let beads_dir = tmp.path().join(".beads");
        std::fs::create_dir_all(&beads_dir).unwrap();
        beads_dir
    }

    // ── detect_backend ────────────────────────────────────────────────────────

    #[test]
    fn detect_returns_jsonl_when_no_db_true() {
        let tmp = tempfile::tempdir().unwrap();
        let beads_dir = make_beads_dir(&tmp);
        std::fs::write(beads_dir.join("config.yaml"), "no-db: true\n").unwrap();

        assert_eq!(detect_backend(tmp.path()), BackendType::Jsonl);
    }

    #[test]
    fn detect_returns_dolt_when_dolt_dir_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let beads_dir = make_beads_dir(&tmp);
        std::fs::create_dir_all(beads_dir.join("dolt")).unwrap();

        assert_eq!(detect_backend(tmp.path()), BackendType::Dolt);
    }

    #[test]
    fn detect_returns_jsonl_fallback_when_no_indicators() {
        let tmp = tempfile::tempdir().unwrap();
        // No .beads directory at all -> fallback
        assert_eq!(detect_backend(tmp.path()), BackendType::Jsonl);
    }

    #[test]
    fn detect_no_db_false_does_not_force_jsonl() {
        // `no-db: false` should not block Dolt detection
        let tmp = tempfile::tempdir().unwrap();
        let beads_dir = make_beads_dir(&tmp);
        std::fs::write(beads_dir.join("config.yaml"), "no-db: false\n").unwrap();
        std::fs::create_dir_all(beads_dir.join("dolt")).unwrap();

        assert_eq!(detect_backend(tmp.path()), BackendType::Dolt);
    }

    #[test]
    fn detect_no_db_true_takes_precedence_over_dolt_dir() {
        // Even if .beads/dolt/ exists, `no-db: true` wins
        let tmp = tempfile::tempdir().unwrap();
        let beads_dir = make_beads_dir(&tmp);
        std::fs::write(beads_dir.join("config.yaml"), "no-db: true\n").unwrap();
        std::fs::create_dir_all(beads_dir.join("dolt")).unwrap();

        assert_eq!(detect_backend(tmp.path()), BackendType::Jsonl);
    }

    #[test]
    fn detect_empty_config_falls_through_to_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let beads_dir = make_beads_dir(&tmp);
        std::fs::write(beads_dir.join("config.yaml"), "").unwrap();

        assert_eq!(detect_backend(tmp.path()), BackendType::Jsonl);
    }

    // ── BackendRegistry ───────────────────────────────────────────────────────

    #[test]
    fn registry_creates_jsonl_backend_for_plain_project() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = BackendRegistry::new();

        // Should succeed -- falls back to real JsonlBackend
        let result = registry.get_or_create(tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn registry_creates_dolt_backend_for_dolt_project() {
        let tmp = tempfile::tempdir().unwrap();
        let beads_dir = make_beads_dir(&tmp);
        std::fs::create_dir_all(beads_dir.join("dolt")).unwrap();

        let mut registry = BackendRegistry::new();
        let result = registry.get_or_create(tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn registry_caches_backend_for_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = BackendRegistry::new();

        let first = registry.get_or_create(tmp.path()).unwrap();
        let second = registry.get_or_create(tmp.path()).unwrap();

        // Both Arcs must point to the same allocation
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn registry_caches_independently_per_path() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let mut registry = BackendRegistry::new();

        let b1 = registry.get_or_create(tmp1.path()).unwrap();
        let b2 = registry.get_or_create(tmp2.path()).unwrap();

        // Different paths -> different Arc allocations
        assert!(!Arc::ptr_eq(&b1, &b2));
    }
}
