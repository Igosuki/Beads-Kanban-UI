//! Backend abstraction layer for beads storage.
//!
//! Defines the [`BeadsBackend`] trait that all storage backends must implement,
//! along with shared error types. This module provides a common interface for
//! reading beads, adding comments, and watching for changes, regardless of
//! whether the underlying storage is JSONL files or a Dolt database.

use std::path::{Path, PathBuf};
use std::pin::Pin;

use async_trait::async_trait;
use futures::stream::Stream;

// Re-export the data types used by route handlers.
// These are the canonical definitions from the existing route modules.
// Comment is re-exported for use by DoltBackend (task yii.7+).
#[allow(unused_imports)]
pub use crate::routes::beads::{Bead, Comment};
pub use crate::routes::watch::FileChangeEvent;

pub mod jsonl;
pub mod detect;
pub mod dolt_server;
pub mod dolt;
pub mod dolt_watcher;

/// Errors that can occur in any beads backend.
// Database and CommandFailed variants are used by DoltBackend (task yii.7+).
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum BeadsError {
    /// An I/O error occurred (e.g., reading a JSONL file).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to parse data (e.g., malformed JSONL or unexpected SQL result).
    #[error("Parse error: {0}")]
    Parse(String),

    /// A database-level error occurred (e.g., SQL query failure).
    #[error("Database error: {0}")]
    Database(String),

    /// The requested bead was not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// An external command (e.g., `bd`) failed.
    #[error("Command failed: {0}")]
    CommandFailed(String),
}

/// Trait that all beads storage backends must implement.
///
/// This is the core abstraction that allows the server to work with different
/// storage systems (JSONL files, Dolt database) through a uniform interface.
///
/// # Implementors
///
/// - `JsonlBackend` -- reads/writes `.beads/issues.jsonl` files directly
/// - `DoltBackend` -- queries a Dolt SQL database, writes via the `bd` CLI
// Route integration is task yii.7; trait is unused until then.
#[allow(dead_code)]
#[async_trait]
pub trait BeadsBackend: Send + Sync {
    /// Read all beads for a given project.
    ///
    /// Returns the full list of beads with their comments, parent-child
    /// relationships, and dependency information already resolved.
    async fn read_beads(&self, project_path: &Path) -> Result<Vec<Bead>, BeadsError>;

    /// Add a comment to a specific bead.
    ///
    /// Returns the updated bead (with the new comment included) on success.
    async fn add_comment(
        &self,
        project_path: &Path,
        bead_id: &str,
        text: &str,
        author: &str,
    ) -> Result<Bead, BeadsError>;

    /// Start watching for changes in the given project.
    ///
    /// Returns a stream of [`FileChangeEvent`]s. The stream is long-lived and
    /// will emit events whenever the underlying data changes (file modification
    /// for JSONL, hash-based polling for Dolt).
    fn watch(&self, project_path: PathBuf) -> Pin<Box<dyn Stream<Item = FileChangeEvent> + Send>>;
}
