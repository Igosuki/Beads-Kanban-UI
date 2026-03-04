//! Dolt hash-based polling watcher.
//!
//! This module provides [`dolt_watch`], a function that returns a stream of
//! [`FileChangeEvent`]s by periodically polling the Dolt `hashof_table()`
//! function and emitting an event whenever the issues table hash changes.
//!
//! Unlike the JSONL watcher (which uses filesystem notifications), this watcher
//! polls the SQL server on a 2-second interval. This is appropriate for Dolt
//! because changes arrive via the `bd` CLI committing to the database rather
//! than through direct file writes.

// Public API items are intentionally defined ahead of their first call sites
// while the DoltBackend integration is being built out incrementally (task yii.5+).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::Stream;
use mysql_async::prelude::*;
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info, warn};

use crate::backend::dolt_server::DoltServerManager;
use crate::backend::FileChangeEvent;

/// Starts a background polling task and returns a stream of [`FileChangeEvent`]s.
///
/// The stream uses hash-based change detection: every 2 seconds it queries
/// `SELECT hashof_table('issues')` against the running Dolt SQL server. When
/// the hash differs from the previous value an event with `change_type =
/// "modified"` is emitted.
///
/// # Initial event
///
/// A `"connected"` event is emitted as soon as the background task starts,
/// before the first poll completes. This mirrors the behaviour of
/// [`JsonlBackend::watch`].
///
/// # Error handling
///
/// - If the server is not yet ready the task waits 5 seconds and retries.
/// - If a SQL query fails the task logs a warning, waits 5 seconds, and
///   continues — the stream is never terminated by transient errors.
/// - The stream ends only when the receiver is dropped (i.e. the client
///   disconnects).
pub fn dolt_watch(
    project_path: PathBuf,
    server_manager: Arc<tokio::sync::Mutex<DoltServerManager>>,
) -> Pin<Box<dyn Stream<Item = FileChangeEvent> + Send>> {
    let (tx, rx) = mpsc::channel::<FileChangeEvent>(100);

    tokio::spawn(async move {
        run_poll_loop(project_path, server_manager, tx).await;
    });

    Box::pin(ReceiverStream::new(rx))
}

/// The long-running polling loop that drives the watcher.
///
/// Separated from [`dolt_watch`] for testability and to keep the spawn closure
/// small.
async fn run_poll_loop(
    project_path: PathBuf,
    server_manager: Arc<tokio::sync::Mutex<DoltServerManager>>,
    tx: mpsc::Sender<FileChangeEvent>,
) {
    // Send initial "connected" event before the first poll.
    let connect_event = FileChangeEvent {
        path: project_path.to_string_lossy().to_string(),
        change_type: "connected".to_string(),
    };
    if tx.send(connect_event).await.is_err() {
        return;
    }
    info!(
        "DoltWatcher: started for project {}",
        project_path.display()
    );

    // Obtain the initial hash.  Retry with a 5-second back-off until the
    // server becomes available.
    let mut previous_hash = loop {
        match query_hash(&project_path, &server_manager).await {
            Some(hash) => break hash,
            None => {
                warn!(
                    "DoltWatcher: server not ready for {}, retrying in 5s",
                    project_path.display()
                );
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    };

    info!(
        "DoltWatcher: initial hash for {} is {}",
        project_path.display(),
        &previous_hash
    );

    let mut poll_interval = interval(Duration::from_secs(2));
    // Consume the first tick that fires immediately.
    poll_interval.tick().await;

    loop {
        poll_interval.tick().await;

        // If the sender is gone (receiver dropped), stop polling.
        if tx.is_closed() {
            info!(
                "DoltWatcher: receiver gone, stopping for {}",
                project_path.display()
            );
            break;
        }

        let current_hash = match query_hash(&project_path, &server_manager).await {
            Some(hash) => hash,
            None => {
                warn!(
                    "DoltWatcher: query failed for {}, waiting 5s before next poll",
                    project_path.display()
                );
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        if current_hash != previous_hash {
            info!(
                "DoltWatcher: hash changed for {} ({} -> {}), emitting modified event",
                project_path.display(),
                &previous_hash,
                &current_hash
            );

            let event = FileChangeEvent {
                path: project_path.to_string_lossy().to_string(),
                change_type: "modified".to_string(),
            };

            if tx.send(event).await.is_err() {
                info!(
                    "DoltWatcher: client disconnected, stopping for {}",
                    project_path.display()
                );
                break;
            }

            previous_hash = current_hash;
        }
    }

    info!("DoltWatcher: poll loop stopped for {}", project_path.display());
}

/// Queries `hashof_table('issues')` and returns the hash string, or `None`
/// on any error.
///
/// Acquires the `DoltServerManager` mutex, fetches a connection, then releases
/// the lock before executing the SQL query so the lock is not held during
/// potentially slow I/O.
async fn query_hash(
    project_path: &Path,
    server_manager: &Arc<tokio::sync::Mutex<DoltServerManager>>,
) -> Option<String> {
    // Acquire the lock only long enough to get a connection, then release it.
    let pool = {
        let mut mgr = server_manager.lock().await;
        match mgr.get_connection(project_path).await {
            Ok(conn) => conn.pool().clone(),
            Err(e) => {
                warn!("DoltWatcher: could not get connection: {}", e);
                return None;
            }
        }
    };

    let mut conn = match pool.get_conn().await {
        Ok(c) => c,
        Err(e) => {
            warn!("DoltWatcher: pool.get_conn() failed: {}", e);
            return None;
        }
    };

    let result: Option<(Option<String>,)> = match conn
        .query_first("SELECT hashof_table('issues') as hash")
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("DoltWatcher: hashof_table query failed: {}", e);
            return None;
        }
    };

    match result {
        Some((Some(hash),)) => Some(hash),
        Some((None,)) => {
            // hashof_table returns NULL when the table doesn't exist yet.
            warn!("DoltWatcher: hashof_table returned NULL (table may not exist yet)");
            None
        }
        None => {
            error!("DoltWatcher: hashof_table query returned no rows");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    /// Helper: create a server manager wrapped in Arc<Mutex<>>.
    fn make_manager() -> Arc<tokio::sync::Mutex<DoltServerManager>> {
        Arc::new(tokio::sync::Mutex::new(DoltServerManager::new()))
    }

    #[tokio::test]
    async fn test_dolt_watch_emits_connected_event_immediately() {
        // The watcher should emit a "connected" event before performing any
        // SQL query.  We use a project path that will never have a Dolt server,
        // so the poll loop will be stuck retrying — but the initial event
        // should still be delivered promptly.
        let dir = tempfile::TempDir::new().unwrap();
        let manager = make_manager();

        let mut stream = dolt_watch(dir.path().to_path_buf(), manager);

        // Wait up to 1 second for the initial connected event.
        use futures::StreamExt;
        let event = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("timed out waiting for initial event")
            .expect("stream ended unexpectedly");

        assert_eq!(event.change_type, "connected");
        assert_eq!(event.path, dir.path().to_string_lossy().as_ref());
    }

    #[tokio::test]
    async fn test_dolt_watch_stream_ends_when_receiver_dropped() {
        // When the stream (receiver) is dropped the background task should
        // detect this via `tx.is_closed()` and stop.  We can't easily assert
        // the task has stopped, but we verify drop does not panic.
        let dir = tempfile::TempDir::new().unwrap();
        let manager = make_manager();

        let stream = dolt_watch(dir.path().to_path_buf(), manager);
        // Drop the stream immediately.
        drop(stream);

        // Give the task a moment to observe `is_closed()` — not strictly
        // necessary but prevents the test from leaving dangling tasks.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_dolt_watch_connected_event_has_project_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let project_path = dir.path().to_path_buf();
        let expected_path = project_path.to_string_lossy().to_string();
        let manager = make_manager();

        let mut stream = dolt_watch(project_path, manager);

        use futures::StreamExt;
        let event = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("timed out")
            .expect("stream empty");

        assert_eq!(event.path, expected_path);
    }
}
