//! Dolt server lifecycle manager.
//!
//! This module manages the lifecycle of `dolt sql-server` child processes and
//! provides MySQL connection pooling via [`mysql_async`].
//!
//! # Overview
//!
//! [`DoltServerManager`] tracks spawned `dolt sql-server` processes, one per
//! project path. When [`DoltServerManager::get_connection`] is called:
//!
//! 1. It looks for an existing server by probing a saved port in
//!    `.beads/dolt/server.json`.
//! 2. If no server is found it picks a free port, spawns `dolt sql-server`,
//!    waits for it to become ready (TCP connect with exponential back-off),
//!    and saves the port.
//! 3. It returns a [`DoltConnection`] that owns a [`mysql_async::Pool`].

// Public API items are intentionally defined ahead of their first call sites
// while the DoltBackend integration is being built out incrementally.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mysql_async::{OptsBuilder, PoolConstraints, PoolOpts};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::sleep;

use crate::backend::BeadsError;

// ---------------------------------------------------------------------------
// Server state persisted next to the dolt database
// ---------------------------------------------------------------------------

/// JSON written to `.beads/dolt/server.json` so future calls can find an
/// already-running server.
#[derive(Debug, Serialize, Deserialize)]
struct ServerState {
    port: u16,
    db_name: String,
}

// ---------------------------------------------------------------------------
// DoltConnection
// ---------------------------------------------------------------------------

/// A live connection to a running `dolt sql-server`, backed by a
/// [`mysql_async::Pool`].
pub struct DoltConnection {
    pub host: String,
    pub port: u16,
    pub database: String,
    pool: mysql_async::Pool,
}

impl DoltConnection {
    /// Returns a reference to the underlying connection pool.
    pub fn pool(&self) -> &mysql_async::Pool {
        &self.pool
    }
}

// ---------------------------------------------------------------------------
// DoltServerManager
// ---------------------------------------------------------------------------

/// Manages `dolt sql-server` child processes, keyed by project path.
///
/// Only one server is started per unique `project_path`. If a server is
/// already running (detected via `.beads/dolt/server.json` and a successful
/// TCP probe) it is reused without spawning a new process.
pub struct DoltServerManager {
    /// Child processes we have spawned (project_path → child).
    children: HashMap<PathBuf, Child>,
}

impl DoltServerManager {
    /// Creates an empty manager with no running servers.
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Returns a [`DoltConnection`] for the given project, starting a
    /// `dolt sql-server` if one is not already running.
    ///
    /// # Errors
    ///
    /// Returns [`BeadsError::CommandFailed`] when:
    /// - the `.beads/dolt/` directory cannot be found or is empty,
    /// - the `dolt` binary is not on `PATH`,
    /// - the server does not become ready within 10 seconds.
    pub async fn get_connection(
        &mut self,
        project_path: &Path,
    ) -> Result<DoltConnection, BeadsError> {
        let (db_dir, db_name) = locate_db_dir(project_path)?;
        let state_path = project_path.join(".beads/dolt/server.json");

        // Check if an already-running server is reachable.
        if let Some(state) = read_server_state(&state_path) {
            if tcp_probe("127.0.0.1", state.port).await {
                return build_connection("127.0.0.1", state.port, &state.db_name);
            }
        }

        // No running server — pick a free port and start one.
        let port = free_port()?;
        self.spawn_server(&db_dir, port).await?;
        wait_for_ready("127.0.0.1", port, Duration::from_secs(10)).await?;

        // Persist the port so the next call can skip spawning.
        let state = ServerState {
            port,
            db_name: db_name.clone(),
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = fs::write(&state_path, json);
        }

        build_connection("127.0.0.1", port, &db_name)
    }

    /// Stops the server that was spawned for `project_path`, if any.
    pub async fn stop(&mut self, project_path: &Path) {
        let key = project_path.to_path_buf();
        if let Some(mut child) = self.children.remove(&key) {
            let _ = child.start_kill();
        }

        // Also remove the persisted server state.
        let state_path = project_path.join(".beads/dolt/server.json");
        let _ = fs::remove_file(state_path);
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Spawns `dolt sql-server` in `db_dir` and stores the child handle.
    async fn spawn_server(&mut self, db_dir: &Path, port: u16) -> Result<(), BeadsError> {
        let child = Command::new("dolt")
            .args([
                "sql-server",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ])
            .current_dir(db_dir)
            // Suppress stdout/stderr so they don't clutter the server log.
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    BeadsError::CommandFailed(
                        "dolt binary not found — please install dolt and ensure it is on PATH"
                            .to_string(),
                    )
                } else {
                    BeadsError::CommandFailed(format!("failed to spawn dolt sql-server: {e}"))
                }
            })?;

        self.children.insert(db_dir.to_path_buf(), child);
        Ok(())
    }
}

impl Drop for DoltServerManager {
    fn drop(&mut self) {
        // Best-effort kill all child processes we own.
        for (_, mut child) in self.children.drain() {
            let _ = child.start_kill();
        }
    }
}

// ---------------------------------------------------------------------------
// Free-standing helpers
// ---------------------------------------------------------------------------

/// Locates the dolt database directory inside `project_path/.beads/dolt/`.
///
/// Returns `(db_dir, db_name)` where `db_name` is the directory name with
/// hyphens replaced by underscores (MySQL database naming convention).
fn locate_db_dir(project_path: &Path) -> Result<(PathBuf, String), BeadsError> {
    let dolt_root = project_path.join(".beads/dolt");

    let entries = fs::read_dir(&dolt_root).map_err(|e| {
        BeadsError::CommandFailed(format!(
            "cannot read dolt directory {}: {e}",
            dolt_root.display()
        ))
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let raw_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            // MySQL database names use underscores, not hyphens.
            let db_name = raw_name.replace('-', "_");
            return Ok((path, db_name));
        }
    }

    Err(BeadsError::CommandFailed(format!(
        "no dolt database directory found under {}",
        dolt_root.display()
    )))
}

/// Reads persisted server state from `path`, returning `None` on any error.
fn read_server_state(path: &Path) -> Option<ServerState> {
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Returns an OS-assigned free TCP port by binding then immediately dropping
/// the listener.
fn free_port() -> Result<u16, BeadsError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| BeadsError::CommandFailed(format!("failed to find free port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| BeadsError::CommandFailed(format!("failed to get local address: {e}")))?
        .port();
    // Drop the listener — port is released and dolt can bind to it.
    drop(listener);
    Ok(port)
}

/// Attempts a single non-blocking TCP connection to `host:port`.
async fn tcp_probe(host: &str, port: u16) -> bool {
    TcpStream::connect(format!("{host}:{port}")).await.is_ok()
}

/// Retries TCP probes with exponential back-off until the server is ready or
/// `timeout` elapses.
///
/// Back-off sequence (ms): 100, 200, 400, 800, 1600, 3200, … capped at 3200 ms.
async fn wait_for_ready(host: &str, port: u16, timeout: Duration) -> Result<(), BeadsError> {
    let start = tokio::time::Instant::now();
    let mut delay_ms: u64 = 100;

    loop {
        if tcp_probe(host, port).await {
            return Ok(());
        }

        if start.elapsed() >= timeout {
            return Err(BeadsError::CommandFailed(format!(
                "dolt sql-server did not become ready on {host}:{port} within {}s",
                timeout.as_secs()
            )));
        }

        sleep(Duration::from_millis(delay_ms)).await;
        delay_ms = (delay_ms * 2).min(3200);
    }
}

/// Builds a [`DoltConnection`] with a pool (min 1, max 5 connections).
fn build_connection(host: &str, port: u16, db_name: &str) -> Result<DoltConnection, BeadsError> {
    let constraints = PoolConstraints::new(1, 5).ok_or_else(|| {
        BeadsError::CommandFailed("invalid pool constraints".to_string())
    })?;
    let pool_opts = PoolOpts::default().with_constraints(constraints);

    let opts = OptsBuilder::default()
        .ip_or_hostname(host)
        .tcp_port(port)
        .db_name(Some(db_name))
        .pool_opts(pool_opts);

    let pool = mysql_async::Pool::new(opts);

    Ok(DoltConnection {
        host: host.to_string(),
        port,
        database: db_name.to_string(),
        pool,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_project_with_db(db_subdir: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join(".beads/dolt").join(db_subdir);
        fs::create_dir_all(&db_path).unwrap();
        dir
    }

    #[test]
    fn test_locate_db_dir_finds_first_subdir() {
        let project = make_project_with_db("beads_MyProject");
        let (db_dir, db_name) = locate_db_dir(project.path()).unwrap();
        assert_eq!(db_name, "beads_MyProject");
        assert!(db_dir.ends_with("beads_MyProject"));
    }

    #[test]
    fn test_locate_db_dir_replaces_hyphens() {
        let project = make_project_with_db("beads_My-Project");
        let (_db_dir, db_name) = locate_db_dir(project.path()).unwrap();
        assert_eq!(db_name, "beads_My_Project");
    }

    #[test]
    fn test_locate_db_dir_missing_returns_error() {
        let dir = TempDir::new().unwrap();
        // No .beads/dolt directory created.
        let result = locate_db_dir(dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            BeadsError::CommandFailed(_) => {}
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }

    #[test]
    fn test_locate_db_dir_empty_dolt_dir_returns_error() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".beads/dolt")).unwrap();
        let result = locate_db_dir(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_free_port_returns_nonzero() {
        let port = free_port().unwrap();
        assert!(port > 0);
    }

    #[test]
    fn test_read_server_state_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("server.json");
        assert!(read_server_state(&path).is_none());
    }

    #[test]
    fn test_read_server_state_valid_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("server.json");
        fs::write(&path, r#"{"port":3307,"db_name":"beads_test"}"#).unwrap();
        let state = read_server_state(&path).unwrap();
        assert_eq!(state.port, 3307);
        assert_eq!(state.db_name, "beads_test");
    }

    #[test]
    fn test_read_server_state_invalid_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("server.json");
        fs::write(&path, "not-json").unwrap();
        assert!(read_server_state(&path).is_none());
    }

    #[tokio::test]
    async fn test_tcp_probe_closed_port_returns_false() {
        // Pick a port then immediately release it; nothing is listening.
        let port = free_port().unwrap();
        assert!(!tcp_probe("127.0.0.1", port).await);
    }

    #[test]
    fn test_dolt_server_manager_new_is_empty() {
        let mgr = DoltServerManager::new();
        assert!(mgr.children.is_empty());
    }
}
