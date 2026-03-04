//! Dolt SQL database backend implementation.
//!
//! Implements [`BeadsBackend`] by querying a `dolt sql-server` instance via
//! MySQL protocol. Reads are done directly via SQL; writes are delegated to
//! the `bd` CLI tool which owns the write path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::Stream;
use mysql_async::prelude::*;
use tokio::sync::Mutex;

use super::{BeadsBackend, BeadsError};
use crate::backend::dolt_server::DoltServerManager;
use crate::routes::beads::{Bead, Comment, Dependency};
use crate::routes::watch::FileChangeEvent;

/// Backend that reads beads from a Dolt SQL database and delegates writes to
/// the `bd` CLI.
///
/// One [`DoltServerManager`] is shared across all calls; it lazily starts a
/// `dolt sql-server` child process for each project and caches the connection.
// Route integration is task yii.7; struct is unused until then.
#[allow(dead_code)]
pub struct DoltBackend {
    server_manager: Arc<Mutex<DoltServerManager>>,
}

#[allow(dead_code)]
impl DoltBackend {
    /// Creates a new `DoltBackend` wrapping the given server manager.
    pub fn new(server_manager: Arc<Mutex<DoltServerManager>>) -> Self {
        Self { server_manager }
    }
}

#[async_trait]
impl BeadsBackend for DoltBackend {
    /// Read all beads from the Dolt database for the given project.
    ///
    /// Executes three SQL queries (issues, comments, dependencies), assembles
    /// the data, and applies the same four post-processing passes as
    /// [`JsonlBackend`](crate::backend::jsonl::JsonlBackend):
    ///
    /// 1. Extract parent-child from explicit `parent-child` dependencies
    /// 2. Infer parent-child from ID dot patterns (e.g. "64n.1" -> "64n")
    /// 3. Populate `children` lists on parent beads
    /// 4. Extract `relates-to` dependencies into the `relates_to` field
    async fn read_beads(&self, project_path: &Path) -> Result<Vec<Bead>, BeadsError> {
        let conn = {
            let mut mgr = self.server_manager.lock().await;
            mgr.get_connection(project_path).await?
        };
        let pool = conn.pool();

        // -- Query 1: Issues --------------------------------------------------
        let mut mysql_conn = pool.get_conn().await.map_err(|e| {
            BeadsError::Database(format!("failed to get connection from pool: {e}"))
        })?;

        let issue_rows: Vec<mysql_async::Row> = mysql_conn
            .query(
                "SELECT id, title, description, status, priority, issue_type, owner, \
                 created_at, created_by, updated_at, closed_at, close_reason, design \
                 FROM issues WHERE status != 'deleted'",
            )
            .await
            .map_err(|e| BeadsError::Database(format!("issues query failed: {e}")))?;

        let mut beads: Vec<Bead> = issue_rows
            .into_iter()
            .map(row_to_bead)
            .collect::<Result<Vec<_>, _>>()?;

        // Build a lookup so we can attach comments efficiently.
        let mut bead_index: HashMap<String, usize> = beads
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id.clone(), i))
            .collect();

        // -- Query 2: Comments ------------------------------------------------
        let comment_rows: Vec<mysql_async::Row> = mysql_conn
            .query("SELECT id, issue_id, author, text, created_at FROM comments ORDER BY id")
            .await
            .map_err(|e| BeadsError::Database(format!("comments query failed: {e}")))?;

        for row in comment_rows {
            let comment = row_to_comment(row)?;
            if let Some(&idx) = bead_index.get(&comment.issue_id) {
                match &mut beads[idx].comments {
                    Some(comments) => comments.push(comment),
                    None => beads[idx].comments = Some(vec![comment]),
                }
            }
        }

        // -- Query 3: Dependencies --------------------------------------------
        let dep_rows: Vec<mysql_async::Row> = mysql_conn
            .query("SELECT issue_id, depends_on_id, type FROM dependencies")
            .await
            .map_err(|e| BeadsError::Database(format!("dependencies query failed: {e}")))?;

        for row in dep_rows {
            let issue_id: String = row
                .get_opt::<String, _>("issue_id")
                .and_then(|r| r.ok())
                .ok_or_else(|| {
                    BeadsError::Parse("missing issue_id in dependencies".to_string())
                })?;
            let depends_on_id: String = row
                .get_opt::<String, _>("depends_on_id")
                .and_then(|r| r.ok())
                .ok_or_else(|| {
                    BeadsError::Parse("missing depends_on_id in dependencies".to_string())
                })?;
            let dep_type: String = row
                .get_opt::<String, _>("type")
                .and_then(|r| r.ok())
                .ok_or_else(|| {
                    BeadsError::Parse("missing type in dependencies".to_string())
                })?;

            if let Some(&idx) = bead_index.get(&issue_id) {
                let dep = Dependency {
                    depends_on_id,
                    dep_type,
                };
                match &mut beads[idx].dependencies {
                    Some(deps) => deps.push(dep),
                    None => beads[idx].dependencies = Some(vec![dep]),
                }
            }
        }

        // Rebuild the index after all mutations (ownership rules require a
        // fresh borrow).
        bead_index = beads
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id.clone(), i))
            .collect();

        // -- Pass 1: Extract parent-child from explicit dependencies ----------
        let mut parent_to_children: HashMap<String, Vec<String>> = HashMap::new();

        for bead in &mut beads {
            if let Some(deps) = &bead.dependencies {
                for dep in deps {
                    if dep.dep_type == "parent-child" {
                        bead.parent_id = Some(dep.depends_on_id.clone());
                        parent_to_children
                            .entry(dep.depends_on_id.clone())
                            .or_default()
                            .push(bead.id.clone());
                    }
                }
            }
        }

        // -- Pass 2: Infer parent-child from ID dot patterns ------------------
        let bead_ids: std::collections::HashSet<String> =
            beads.iter().map(|b| b.id.clone()).collect();

        let inferred: Vec<(String, String)> = beads
            .iter()
            .filter_map(|bead| {
                if bead.parent_id.is_some() {
                    return None;
                }
                let dot_pos = bead.id.rfind('.')?;
                let potential_parent = &bead.id[..dot_pos];
                if bead_ids.contains(potential_parent) {
                    Some((bead.id.clone(), potential_parent.to_string()))
                } else {
                    None
                }
            })
            .collect();

        for (child_id, inferred_parent_id) in &inferred {
            if let Some(&idx) = bead_index.get(child_id) {
                beads[idx].parent_id = Some(inferred_parent_id.clone());
            }
            parent_to_children
                .entry(inferred_parent_id.clone())
                .or_default()
                .push(child_id.clone());
        }

        // -- Pass 3: Populate children lists on parent beads ------------------
        for bead in &mut beads {
            if let Some(children) = parent_to_children.get(&bead.id) {
                bead.children = Some(children.clone());
            }
        }

        // -- Pass 4: Extract relates-to into relates_to field -----------------
        for bead in &mut beads {
            if let Some(deps) = &bead.dependencies {
                let related: Vec<String> = deps
                    .iter()
                    .filter(|dep| dep.dep_type == "relates-to")
                    .map(|dep| dep.depends_on_id.clone())
                    .collect();
                if !related.is_empty() {
                    bead.relates_to = Some(related);
                }
            }
        }

        Ok(beads)
    }

    /// Add a comment to the specified bead using the `bd` CLI.
    ///
    /// Shells out to `bd comments add <bead_id> --text <text> --author
    /// <author>` in the project directory, then re-reads the bead from SQL to
    /// return the fully updated state.
    async fn add_comment(
        &self,
        project_path: &Path,
        bead_id: &str,
        text: &str,
        author: &str,
    ) -> Result<Bead, BeadsError> {
        // Delegate the write to the `bd` CLI so that it owns all Dolt commit
        // semantics (branch, author metadata, etc.).
        let output = tokio::process::Command::new("bd")
            .args(["comments", "add", bead_id, "--text", text, "--author", author])
            .current_dir(project_path)
            .output()
            .await
            .map_err(|e| {
                BeadsError::CommandFailed(format!("failed to spawn `bd comments add`: {e}"))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BeadsError::CommandFailed(format!(
                "`bd comments add` failed: {stderr}"
            )));
        }

        // Re-read from SQL to get the fully updated state including the new
        // comment (the CLI may have modified other fields too).
        let beads = self.read_beads(project_path).await?;
        beads
            .into_iter()
            .find(|b| b.id == bead_id)
            .ok_or_else(|| {
                BeadsError::NotFound(format!("bead '{bead_id}' not found after comment add"))
            })
    }

    /// Watch stub -- hash-based polling watcher is implemented in task yii.6.
    fn watch(
        &self,
        _project_path: PathBuf,
    ) -> Pin<Box<dyn Stream<Item = FileChangeEvent> + Send>> {
        Box::pin(futures::stream::empty())
    }
}

// ---------------------------------------------------------------------------
// Row mapping helpers
// ---------------------------------------------------------------------------

/// Map a MySQL `issues` row to a [`Bead`].
fn row_to_bead(row: mysql_async::Row) -> Result<Bead, BeadsError> {
    let id: String = row
        .get_opt::<String, _>("id")
        .and_then(|r| r.ok())
        .ok_or_else(|| BeadsError::Parse("missing id column in issues".to_string()))?;

    let title: String = row
        .get_opt::<String, _>("title")
        .and_then(|r| r.ok())
        .ok_or_else(|| BeadsError::Parse(format!("missing title for bead '{id}'")))?;

    let status: String = row
        .get_opt::<String, _>("status")
        .and_then(|r| r.ok())
        .ok_or_else(|| BeadsError::Parse(format!("missing status for bead '{id}'")))?;

    let description: Option<String> = row
        .get_opt::<String, _>("description")
        .and_then(|r| r.ok());

    let priority: Option<i32> = row.get_opt::<i32, _>("priority").and_then(|r| r.ok());

    let issue_type: Option<String> = row
        .get_opt::<String, _>("issue_type")
        .and_then(|r| r.ok());

    let owner: Option<String> = row.get_opt::<String, _>("owner").and_then(|r| r.ok());

    let created_at: Option<String> = row
        .get_opt::<String, _>("created_at")
        .and_then(|r| r.ok());

    let created_by: Option<String> = row
        .get_opt::<String, _>("created_by")
        .and_then(|r| r.ok());

    let updated_at: Option<String> = row
        .get_opt::<String, _>("updated_at")
        .and_then(|r| r.ok());

    let closed_at: Option<String> = row
        .get_opt::<String, _>("closed_at")
        .and_then(|r| r.ok());

    let close_reason: Option<String> = row
        .get_opt::<String, _>("close_reason")
        .and_then(|r| r.ok());

    // SQL column is "design"; Bead field is `design_doc`.
    let design_doc: Option<String> = row.get_opt::<String, _>("design").and_then(|r| r.ok());

    Ok(Bead {
        id,
        title,
        description,
        status,
        priority,
        issue_type,
        owner,
        created_at,
        created_by,
        updated_at,
        closed_at,
        close_reason,
        comments: None,
        parent_id: None,
        children: None,
        design_doc,
        // `deps` (blocks) and `relates_to` are populated after the
        // dependencies query in `read_beads`.
        deps: None,
        relates_to: None,
        dependencies: None,
    })
}

/// Map a MySQL `comments` row to a [`Comment`].
fn row_to_comment(row: mysql_async::Row) -> Result<Comment, BeadsError> {
    let id: i64 = row
        .get_opt::<i64, _>("id")
        .and_then(|r| r.ok())
        .ok_or_else(|| BeadsError::Parse("missing id column in comments".to_string()))?;

    let issue_id: String = row
        .get_opt::<String, _>("issue_id")
        .and_then(|r| r.ok())
        .ok_or_else(|| {
            BeadsError::Parse("missing issue_id column in comments".to_string())
        })?;

    let author: String = row
        .get_opt::<String, _>("author")
        .and_then(|r| r.ok())
        .ok_or_else(|| BeadsError::Parse(format!("missing author for comment id '{id}'")))?;

    let text: String = row
        .get_opt::<String, _>("text")
        .and_then(|r| r.ok())
        .ok_or_else(|| BeadsError::Parse(format!("missing text for comment id '{id}'")))?;

    let created_at: String = row
        .get_opt::<String, _>("created_at")
        .and_then(|r| r.ok())
        .ok_or_else(|| {
            BeadsError::Parse(format!("missing created_at for comment id '{id}'"))
        })?;

    Ok(Comment {
        id,
        issue_id,
        author,
        text,
        created_at,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Construction must not panic.
    #[test]
    fn test_dolt_backend_new_does_not_panic() {
        let manager = Arc::new(Mutex::new(DoltServerManager::new()));
        let _backend = DoltBackend::new(manager);
    }

    /// The watch stub must return an empty stream immediately.
    #[tokio::test]
    async fn test_watch_returns_empty_stream() {
        use futures::StreamExt;

        let manager = Arc::new(Mutex::new(DoltServerManager::new()));
        let backend = DoltBackend::new(manager);
        let mut stream = backend.watch(PathBuf::from("/tmp/test-project"));
        assert!(stream.next().await.is_none());
    }
}
