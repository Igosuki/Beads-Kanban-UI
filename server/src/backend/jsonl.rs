//! JSONL file-based backend implementation.
//!
//! Implements [`BeadsBackend`] by reading and writing `.beads/issues.jsonl`
//! files directly. This is the original storage format for beads projects.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::Stream;
use notify::{event::ModifyKind, Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info, warn};

use super::{BeadsBackend, BeadsError};
use crate::routes::beads::{recompute_epic_statuses, resolve_issues_path, Bead, Comment};
use crate::routes::watch::FileChangeEvent;

/// Backend that reads and writes beads data as JSONL files.
///
/// Each bead is stored as a single JSON object on its own line in
/// `.beads/issues.jsonl` (or the sync-branch equivalent resolved by
/// [`resolve_issues_path`]).
// Route integration is task yii.7; struct is unused until then.
#[allow(dead_code)]
pub struct JsonlBackend;

#[async_trait]
impl BeadsBackend for JsonlBackend {
    /// Read all beads from the JSONL file for the given project.
    ///
    /// Performs JSONL parsing followed by four post-processing passes:
    /// 1. Extract parent-child relationships from explicit `dependencies`
    /// 2. Infer parent-child from ID dot patterns (e.g. "64n.1" -> "64n")
    /// 3. Populate `children` lists on parent beads
    /// 4. Extract `relates-to` dependencies into the `relates_to` field
    async fn read_beads(&self, project_path: &Path) -> Result<Vec<Bead>, BeadsError> {
        let issues_path = resolve_issues_path(project_path);

        if !issues_path.exists() {
            return Err(BeadsError::NotFound(
                "No .beads/issues.jsonl found at the specified path".to_string(),
            ));
        }

        let contents = std::fs::read_to_string(&issues_path)?;

        // Parse JSONL — each non-empty line is a JSON object.
        let mut beads = Vec::new();
        for (line_num, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Bead>(line) {
                Ok(bead) => beads.push(bead),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse bead at line {}: {} - {}",
                        line_num + 1,
                        e,
                        line
                    );
                    // Continue — graceful handling of malformed lines.
                }
            }
        }

        // --- Pass 1: Extract parent-child from explicit dependencies ---
        let mut parent_to_children: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();

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

        // --- Pass 2: Infer parent-child from ID dot patterns ---
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
            if let Some(bead) = beads.iter_mut().find(|b| &b.id == child_id) {
                bead.parent_id = Some(inferred_parent_id.clone());
            }
            parent_to_children
                .entry(inferred_parent_id.clone())
                .or_default()
                .push(child_id.clone());
        }

        // --- Pass 3: Populate children lists on parent beads ---
        for bead in &mut beads {
            if let Some(children) = parent_to_children.get(&bead.id) {
                bead.children = Some(children.clone());
            }
        }

        // --- Pass 4: Extract relates-to into relates_to field ---
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

    /// Add a comment to the specified bead and persist the change.
    ///
    /// Reads the JSONL file, locates the target bead, appends the new comment
    /// with an auto-incremented ID (global max across all beads + 1), then
    /// rewrites the entire file. Returns the updated bead on success.
    async fn add_comment(
        &self,
        project_path: &Path,
        bead_id: &str,
        text: &str,
        author: &str,
    ) -> Result<Bead, BeadsError> {
        let issues_path = resolve_issues_path(project_path);

        if !issues_path.exists() {
            return Err(BeadsError::NotFound(
                "No .beads/issues.jsonl found at the specified path".to_string(),
            ));
        }

        let contents = std::fs::read_to_string(&issues_path)?;

        let mut beads: Vec<Bead> = Vec::new();
        let mut found_bead_index: Option<usize> = None;
        let mut max_comment_id: i64 = 0;

        for (line_num, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Bead>(line) {
                Ok(bead) => {
                    // Track global maximum comment ID.
                    if let Some(comments) = &bead.comments {
                        for comment in comments {
                            if comment.id > max_comment_id {
                                max_comment_id = comment.id;
                            }
                        }
                    }
                    if bead.id == bead_id {
                        found_bead_index = Some(beads.len());
                    }
                    beads.push(bead);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse bead at line {}: {} - {}",
                        line_num + 1,
                        e,
                        line
                    );
                }
            }
        }

        let bead_index = found_bead_index.ok_or_else(|| {
            BeadsError::NotFound(format!("Bead with id '{}' not found", bead_id))
        })?;

        let new_comment = Comment {
            id: max_comment_id + 1,
            issue_id: bead_id.to_string(),
            author: author.to_string(),
            text: text.to_string(),
            created_at: Utc::now().to_rfc3339(),
        };

        let bead = &mut beads[bead_index];
        match &mut bead.comments {
            Some(comments) => comments.push(new_comment),
            None => bead.comments = Some(vec![new_comment]),
        }

        // Rewrite the entire JSONL file.
        let file = std::fs::File::create(&issues_path)?;
        let mut writer = std::io::BufWriter::new(file);
        for bead in &beads {
            let json_line = serde_json::to_string(bead)
                .map_err(|e| BeadsError::Parse(format!("Failed to serialize bead: {}", e)))?;
            writeln!(writer, "{}", json_line)?;
        }
        writer.flush()?;

        Ok(beads.swap_remove(bead_index))
    }

    /// Watch the project's `.beads` directory for file changes.
    ///
    /// Sets up a [`notify::RecommendedWatcher`] on the `.beads` directory,
    /// debounces rapid events, recomputes epic statuses on modifications, and
    /// streams [`FileChangeEvent`]s to the caller.
    fn watch(&self, project_path: PathBuf) -> Pin<Box<dyn Stream<Item = FileChangeEvent> + Send>> {
        let (tx, rx) = mpsc::channel::<FileChangeEvent>(100);

        tokio::spawn(async move {
            let beads_file = resolve_issues_path(&project_path);
            if let Err(e) = run_watcher(beads_file, tx).await {
                error!("JsonlBackend file watcher error: {}", e);
            }
        });

        Box::pin(ReceiverStream::new(rx))
    }
}

/// Internal watcher task: watches the `.beads` directory and forwards
/// debounced [`FileChangeEvent`]s through `tx`.
#[allow(dead_code)]
async fn run_watcher(
    beads_file: PathBuf,
    tx: mpsc::Sender<FileChangeEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (notify_tx, mut notify_rx) = mpsc::channel(100);

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let _ = notify_tx.blocking_send(event);
            }
        },
        Config::default().with_poll_interval(Duration::from_millis(100)),
    )?;

    // Watch the parent directory (.beads) so we catch the file even if it
    // doesn't exist yet.
    let watch_path = beads_file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| beads_file.clone());

    if !watch_path.exists() {
        warn!(
            "Watch path does not exist, waiting for creation: {:?}",
            watch_path
        );
    }

    let actual_watch_path = if watch_path.exists() {
        watch_path.clone()
    } else if let Some(parent) = watch_path.parent() {
        if parent.exists() {
            parent.to_path_buf()
        } else {
            error!("Neither watch path nor parent exists: {:?}", watch_path);
            return Ok(());
        }
    } else {
        error!("No valid path to watch: {:?}", watch_path);
        return Ok(());
    };

    watcher.watch(&actual_watch_path, RecursiveMode::Recursive)?;
    info!("JsonlBackend file watcher active on: {:?}", actual_watch_path);

    // Send initial connection event.
    let connect_event = FileChangeEvent {
        path: beads_file.to_string_lossy().to_string(),
        change_type: "connected".to_string(),
    };
    if tx.send(connect_event).await.is_err() {
        return Ok(());
    }

    // Debounce state.
    let mut last_event_time = std::time::Instant::now();
    let debounce_duration = Duration::from_millis(100);

    while let Some(event) = notify_rx.recv().await {
        let is_relevant = event.paths.iter().any(|p| {
            p.ends_with("issues.jsonl") || p.ends_with(".beads") || p == &beads_file
        });

        if !is_relevant {
            continue;
        }

        let now = std::time::Instant::now();
        if now.duration_since(last_event_time) < debounce_duration {
            continue;
        }
        last_event_time = now;

        let change_type = match event.kind {
            EventKind::Create(_) => "created",
            EventKind::Modify(ModifyKind::Data(_)) => "modified",
            EventKind::Modify(_) => "modified",
            EventKind::Remove(_) => "removed",
            _ => continue,
        };

        let file_event = FileChangeEvent {
            path: beads_file.to_string_lossy().to_string(),
            change_type: change_type.to_string(),
        };

        info!("JsonlBackend file change detected: {:?}", file_event);

        if change_type == "modified" || change_type == "created" {
            match recompute_epic_statuses(&beads_file) {
                Ok(updated_epics) => {
                    if !updated_epics.is_empty() {
                        info!("Updated epic statuses: {:?}", updated_epics);
                    }
                }
                Err(e) => {
                    warn!("Failed to recompute epic statuses: {}", e);
                }
            }
        }

        if tx.send(file_event).await.is_err() {
            info!("JsonlBackend watcher: client gone, stopping");
            break;
        }
    }

    info!("JsonlBackend file watcher stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_jsonl(dir: &Path, lines: &[&str]) {
        let path = dir.join(".beads").join("issues.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }

    // ── read_beads ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_read_beads_returns_not_found_when_no_file() {
        let tmp = tempdir().unwrap();
        let backend = JsonlBackend;
        let result = backend.read_beads(tmp.path()).await;
        assert!(matches!(result, Err(BeadsError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_read_beads_parses_simple_bead() {
        let tmp = tempdir().unwrap();
        write_jsonl(
            tmp.path(),
            &[r#"{"id":"abc","title":"Test","status":"open"}"#],
        );

        let backend = JsonlBackend;
        let beads = backend.read_beads(tmp.path()).await.unwrap();
        assert_eq!(beads.len(), 1);
        assert_eq!(beads[0].id, "abc");
        assert_eq!(beads[0].title, "Test");
        assert_eq!(beads[0].status, "open");
    }

    #[tokio::test]
    async fn test_read_beads_skips_empty_lines() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join(".beads").join("issues.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "\n{\"id\":\"x\",\"title\":\"X\",\"status\":\"open\"}\n\n",
        )
        .unwrap();

        let backend = JsonlBackend;
        let beads = backend.read_beads(tmp.path()).await.unwrap();
        assert_eq!(beads.len(), 1);
    }

    #[tokio::test]
    async fn test_read_beads_infers_parent_from_id_dot_pattern() {
        let tmp = tempdir().unwrap();
        write_jsonl(
            tmp.path(),
            &[
                r#"{"id":"epic1","title":"Epic","status":"open","issue_type":"epic"}"#,
                r#"{"id":"epic1.1","title":"Child","status":"open"}"#,
            ],
        );

        let backend = JsonlBackend;
        let beads = backend.read_beads(tmp.path()).await.unwrap();
        let child = beads.iter().find(|b| b.id == "epic1.1").unwrap();
        assert_eq!(child.parent_id.as_deref(), Some("epic1"));
    }

    #[tokio::test]
    async fn test_read_beads_sets_children_on_parent() {
        let tmp = tempdir().unwrap();
        write_jsonl(
            tmp.path(),
            &[
                r#"{"id":"parent","title":"Parent","status":"open"}"#,
                r#"{"id":"parent.1","title":"Child","status":"open"}"#,
            ],
        );

        let backend = JsonlBackend;
        let beads = backend.read_beads(tmp.path()).await.unwrap();
        let parent = beads.iter().find(|b| b.id == "parent").unwrap();
        let children = parent.children.as_ref().unwrap();
        assert_eq!(children, &["parent.1"]);
    }

    #[tokio::test]
    async fn test_read_beads_extracts_relates_to() {
        let tmp = tempdir().unwrap();
        write_jsonl(
            tmp.path(),
            &[r#"{"id":"a","title":"A","status":"open","dependencies":[{"depends_on_id":"b","type":"relates-to"}]}"#],
        );

        let backend = JsonlBackend;
        let beads = backend.read_beads(tmp.path()).await.unwrap();
        let bead = beads.iter().find(|b| b.id == "a").unwrap();
        let relates = bead.relates_to.as_ref().unwrap();
        assert_eq!(relates, &["b"]);
    }

    #[tokio::test]
    async fn test_read_beads_gracefully_skips_malformed_lines() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join(".beads").join("issues.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "{\"id\":\"good\",\"title\":\"Good\",\"status\":\"open\"}\nnot-valid-json\n",
        )
        .unwrap();

        let backend = JsonlBackend;
        let beads = backend.read_beads(tmp.path()).await.unwrap();
        assert_eq!(beads.len(), 1);
        assert_eq!(beads[0].id, "good");
    }

    #[tokio::test]
    async fn test_read_beads_empty_file_returns_empty_vec() {
        // An existing but totally empty issues.jsonl must return Ok([]) not an error.
        let tmp = tempdir().unwrap();
        let path = tmp.path().join(".beads").join("issues.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();

        let backend = JsonlBackend;
        let beads = backend.read_beads(tmp.path()).await.unwrap();
        assert!(beads.is_empty());
    }

    // ── add_comment ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_add_comment_returns_not_found_when_no_file() {
        let tmp = tempdir().unwrap();
        let backend = JsonlBackend;
        let result = backend
            .add_comment(tmp.path(), "abc", "hello", "user@example.com")
            .await;
        assert!(matches!(result, Err(BeadsError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_add_comment_returns_not_found_for_missing_bead() {
        let tmp = tempdir().unwrap();
        write_jsonl(
            tmp.path(),
            &[r#"{"id":"exists","title":"Exists","status":"open"}"#],
        );

        let backend = JsonlBackend;
        let result = backend
            .add_comment(tmp.path(), "does-not-exist", "text", "author")
            .await;
        assert!(matches!(result, Err(BeadsError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_add_comment_appends_comment_to_bead() {
        let tmp = tempdir().unwrap();
        write_jsonl(
            tmp.path(),
            &[r#"{"id":"bead1","title":"Bead","status":"open"}"#],
        );

        let backend = JsonlBackend;
        let updated = backend
            .add_comment(tmp.path(), "bead1", "Great work!", "alice@example.com")
            .await
            .unwrap();

        let comments = updated.comments.as_ref().unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "Great work!");
        assert_eq!(comments[0].author, "alice@example.com");
        assert_eq!(comments[0].issue_id, "bead1");
    }

    #[tokio::test]
    async fn test_add_comment_increments_global_max_comment_id() {
        let tmp = tempdir().unwrap();
        write_jsonl(
            tmp.path(),
            &[
                r#"{"id":"a","title":"A","status":"open","comments":[{"id":5,"issue_id":"a","author":"u","text":"t","created_at":"2026-01-01T00:00:00Z"}]}"#,
                r#"{"id":"b","title":"B","status":"open"}"#,
            ],
        );

        let backend = JsonlBackend;
        let updated = backend
            .add_comment(tmp.path(), "b", "new comment", "user")
            .await
            .unwrap();

        let comments = updated.comments.as_ref().unwrap();
        assert_eq!(comments[0].id, 6); // global max was 5, so new is 6
    }

    #[tokio::test]
    async fn test_add_comment_persists_to_file() {
        let tmp = tempdir().unwrap();
        write_jsonl(
            tmp.path(),
            &[r#"{"id":"bead1","title":"Bead","status":"open"}"#],
        );

        let backend = JsonlBackend;
        backend
            .add_comment(tmp.path(), "bead1", "persisted", "user")
            .await
            .unwrap();

        // Re-read and verify the comment is on disk.
        let beads = backend.read_beads(tmp.path()).await.unwrap();
        let bead = beads.iter().find(|b| b.id == "bead1").unwrap();
        let comments = bead.comments.as_ref().unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "persisted");
    }
}
