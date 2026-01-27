//! Memory API route handlers.
//!
//! Provides endpoints for reading, editing, and deleting knowledge base entries
//! from `.beads/memory/knowledge.jsonl` files.

use axum::{
    extract::Query,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::validate_path_security;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single memory/knowledge entry from the JSONL file.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryEntry {
    pub key: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub content: String,
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub ts: i64,
    #[serde(default)]
    pub bead: String,
}

/// Aggregated statistics about memory entries.
#[derive(Debug, Serialize)]
pub struct MemoryStats {
    pub total: usize,
    pub learned: usize,
    pub investigation: usize,
    pub archived: usize,
}

/// Response for the list memory endpoint.
#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub entries: Vec<MemoryEntry>,
    pub stats: MemoryStats,
}

/// Query parameters for GET endpoints.
#[derive(Debug, Deserialize)]
pub struct MemoryParams {
    pub path: String,
}

/// Request body for the update memory endpoint.
#[derive(Debug, Deserialize)]
pub struct UpdateMemoryRequest {
    pub path: String,
    pub key: String,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
}

/// Request body for the delete memory endpoint.
#[derive(Debug, Deserialize)]
pub struct DeleteMemoryRequest {
    pub path: String,
    pub key: String,
    #[serde(default)]
    pub archive: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the path to the active knowledge file.
fn knowledge_path(project_path: &Path) -> PathBuf {
    project_path
        .join(".beads")
        .join("memory")
        .join("knowledge.jsonl")
}

/// Build the path to the archive knowledge file.
fn archive_path(project_path: &Path) -> PathBuf {
    project_path
        .join(".beads")
        .join("memory")
        .join("knowledge.archive.jsonl")
}

/// Parse a JSONL file into a list of `MemoryEntry` values.
///
/// Missing files are treated as empty. Malformed lines are skipped with a
/// warning logged via `tracing`.
fn read_entries(path: &PathBuf) -> Result<Vec<MemoryEntry>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let mut entries = Vec::new();
    for (line_num, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match serde_json::from_str::<MemoryEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!(
                    "Failed to parse memory entry at line {}: {} - {}",
                    line_num + 1,
                    e,
                    line
                );
            }
        }
    }

    Ok(entries)
}

/// Write a list of entries back to a JSONL file (overwrite).
fn write_entries(path: &PathBuf, entries: &[MemoryEntry]) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("Failed to open file for writing: {}", e))?;

    let mut writer = std::io::BufWriter::new(file);
    for entry in entries {
        let json_line = serde_json::to_string(entry)
            .map_err(|e| format!("Failed to serialize entry: {}", e))?;
        writeln!(writer, "{}", json_line)
            .map_err(|e| format!("Failed to write to file: {}", e))?;
    }
    writer
        .flush()
        .map_err(|e| format!("Failed to flush file: {}", e))?;

    Ok(())
}

/// Append a single entry to a JSONL file (creating the file if needed).
fn append_entry(path: &PathBuf, entry: &MemoryEntry) -> Result<(), String> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("Failed to open archive file: {}", e))?;

    let mut writer = std::io::BufWriter::new(file);
    let json_line = serde_json::to_string(entry)
        .map_err(|e| format!("Failed to serialize entry: {}", e))?;
    writeln!(writer, "{}", json_line)
        .map_err(|e| format!("Failed to write to archive: {}", e))?;
    writer
        .flush()
        .map_err(|e| format!("Failed to flush archive: {}", e))?;

    Ok(())
}

/// Count the number of entries in a JSONL file (for archive stats).
fn count_entries(path: &PathBuf) -> usize {
    if !path.exists() {
        return 0;
    }

    match std::fs::read_to_string(path) {
        Ok(contents) => contents
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty()
            })
            .count(),
        Err(_) => 0,
    }
}

/// Compute stats from a list of entries plus an archived count.
fn compute_stats(entries: &[MemoryEntry], archived: usize) -> MemoryStats {
    let learned = entries.iter().filter(|e| e.entry_type == "learned").count();
    let investigation = entries
        .iter()
        .filter(|e| e.entry_type == "investigation")
        .count();

    MemoryStats {
        total: entries.len(),
        learned,
        investigation,
        archived,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/memory?path={project_path}
///
/// Reads all entries from the active knowledge file and returns them along
/// with aggregate statistics. Entries are sorted by `ts` descending (newest
/// first).
pub async fn list_memory(Query(params): Query<MemoryParams>) -> impl IntoResponse {
    let project_path = PathBuf::from(&params.path);

    // Security: Validate path is within allowed directories
    if let Err(e) = validate_path_security(&project_path) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": e })),
        );
    }

    let kpath = knowledge_path(&project_path);
    let apath = archive_path(&project_path);

    let mut entries = match read_entries(&kpath) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            );
        }
    };

    // Sort by ts descending (newest first)
    entries.sort_by(|a, b| b.ts.cmp(&a.ts));

    let archived = count_entries(&apath);
    let stats = compute_stats(&entries, archived);

    (
        StatusCode::OK,
        Json(serde_json::json!(MemoryListResponse { entries, stats })),
    )
}

/// GET /api/memory/stats?path={project_path}
///
/// Lightweight endpoint returning only aggregate statistics (no entry content).
pub async fn memory_stats(Query(params): Query<MemoryParams>) -> impl IntoResponse {
    let project_path = PathBuf::from(&params.path);

    // Security: Validate path is within allowed directories
    if let Err(e) = validate_path_security(&project_path) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": e })),
        );
    }

    let kpath = knowledge_path(&project_path);
    let apath = archive_path(&project_path);

    let entries = match read_entries(&kpath) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            );
        }
    };

    let archived = count_entries(&apath);
    let stats = compute_stats(&entries, archived);

    (StatusCode::OK, Json(serde_json::json!(stats)))
}

/// PUT /api/memory
///
/// Edit an existing entry by key. Updates `content` and/or `tags` fields.
/// At least one of `content` or `tags` must be provided.
/// The `ts` field is NOT updated (it represents original creation time).
pub async fn update_memory(Json(payload): Json<UpdateMemoryRequest>) -> impl IntoResponse {
    // Validate that at least one field is provided
    if payload.content.is_none() && payload.tags.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "At least one of 'content' or 'tags' must be provided"
            })),
        );
    }

    let project_path = PathBuf::from(&payload.path);

    // Security: Validate path is within allowed directories
    if let Err(e) = validate_path_security(&project_path) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": e })),
        );
    }

    let kpath = knowledge_path(&project_path);

    let mut entries = match read_entries(&kpath) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            );
        }
    };

    // Find the entry with the matching key
    let entry_pos = entries.iter().position(|e| e.key == payload.key);

    let idx = match entry_pos {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("Entry with key '{}' not found", payload.key)
                })),
            );
        }
    };

    // Update fields
    if let Some(content) = payload.content {
        entries[idx].content = content;
    }
    if let Some(tags) = payload.tags {
        entries[idx].tags = tags;
    }

    // Write back
    if let Err(e) = write_entries(&kpath, &entries) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        );
    }

    let updated_entry = entries[idx].clone();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "entry": updated_entry
        })),
    )
}

/// DELETE /api/memory
///
/// Remove or archive an entry by key.
///
/// - `archive: true`  — Move entry to `knowledge.archive.jsonl`, then remove
///   from `knowledge.jsonl`.
/// - `archive: false` — Permanently delete from `knowledge.jsonl`.
pub async fn delete_memory(Json(payload): Json<DeleteMemoryRequest>) -> impl IntoResponse {
    let project_path = PathBuf::from(&payload.path);

    // Security: Validate path is within allowed directories
    if let Err(e) = validate_path_security(&project_path) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": e })),
        );
    }

    let kpath = knowledge_path(&project_path);

    let mut entries = match read_entries(&kpath) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            );
        }
    };

    // Find the entry with the matching key
    let entry_pos = entries.iter().position(|e| e.key == payload.key);

    let idx = match entry_pos {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("Entry with key '{}' not found", payload.key)
                })),
            );
        }
    };

    // Remove the entry
    let removed_entry = entries.remove(idx);

    // If archiving, append to archive file
    if payload.archive {
        let apath = archive_path(&project_path);
        if let Err(e) = append_entry(&apath, &removed_entry) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            );
        }
    }

    // Write back the remaining entries
    if let Err(e) = write_entries(&kpath, &entries) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "archived": payload.archive
        })),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_memory_entry() {
        let json = r#"{"key":"test-key","type":"learned","content":"Some content","source":"orchestrator","tags":["tag1","tag2"],"ts":1769505562,"bead":"project-id.3"}"#;
        let entry: MemoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.key, "test-key");
        assert_eq!(entry.entry_type, "learned");
        assert_eq!(entry.content, "Some content");
        assert_eq!(entry.source, "orchestrator");
        assert_eq!(entry.tags, vec!["tag1", "tag2"]);
        assert_eq!(entry.ts, 1769505562);
        assert_eq!(entry.bead, "project-id.3");
    }

    #[test]
    fn test_parse_memory_entry_investigation() {
        let json = r#"{"key":"inv-key","type":"investigation","content":"Root cause analysis","source":"detective","tags":["investigation"],"ts":1769505000,"bead":"bd-42.1"}"#;
        let entry: MemoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.entry_type, "investigation");
    }

    #[test]
    fn test_parse_memory_entry_defaults() {
        // tags and bead are optional (have defaults)
        let json = r#"{"key":"min-key","type":"learned","content":"Minimal","source":"src","ts":100}"#;
        let entry: MemoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.tags, Vec::<String>::new());
        assert_eq!(entry.bead, "");
    }

    #[test]
    fn test_serialize_memory_entry() {
        let entry = MemoryEntry {
            key: "k".to_string(),
            entry_type: "learned".to_string(),
            content: "c".to_string(),
            source: "s".to_string(),
            tags: vec!["t".to_string()],
            ts: 42,
            bead: "b".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        // The field should serialize as "type", not "entry_type"
        assert!(json.contains(r#""type":"learned""#));
        assert!(!json.contains("entry_type"));
    }

    #[test]
    fn test_compute_stats() {
        let entries = vec![
            MemoryEntry {
                key: "a".into(),
                entry_type: "learned".into(),
                content: "".into(),
                source: "".into(),
                tags: vec![],
                ts: 1,
                bead: "".into(),
            },
            MemoryEntry {
                key: "b".into(),
                entry_type: "learned".into(),
                content: "".into(),
                source: "".into(),
                tags: vec![],
                ts: 2,
                bead: "".into(),
            },
            MemoryEntry {
                key: "c".into(),
                entry_type: "investigation".into(),
                content: "".into(),
                source: "".into(),
                tags: vec![],
                ts: 3,
                bead: "".into(),
            },
        ];

        let stats = compute_stats(&entries, 5);
        assert_eq!(stats.total, 3);
        assert_eq!(stats.learned, 2);
        assert_eq!(stats.investigation, 1);
        assert_eq!(stats.archived, 5);
    }

    #[test]
    fn test_compute_stats_empty() {
        let stats = compute_stats(&[], 0);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.learned, 0);
        assert_eq!(stats.investigation, 0);
        assert_eq!(stats.archived, 0);
    }

    #[test]
    fn test_read_entries_missing_file() {
        let path = PathBuf::from("/nonexistent/path/knowledge.jsonl");
        let result = read_entries(&path);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_count_entries_missing_file() {
        let path = PathBuf::from("/nonexistent/path/knowledge.jsonl");
        assert_eq!(count_entries(&path), 0);
    }

    #[test]
    fn test_write_and_read_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");

        let entries = vec![
            MemoryEntry {
                key: "k1".into(),
                entry_type: "learned".into(),
                content: "content1".into(),
                source: "src".into(),
                tags: vec!["a".into()],
                ts: 100,
                bead: "b1".into(),
            },
            MemoryEntry {
                key: "k2".into(),
                entry_type: "investigation".into(),
                content: "content2".into(),
                source: "src".into(),
                tags: vec![],
                ts: 200,
                bead: "b2".into(),
            },
        ];

        write_entries(&path, &entries).unwrap();
        let read_back = read_entries(&path).unwrap();

        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[0].key, "k1");
        assert_eq!(read_back[0].entry_type, "learned");
        assert_eq!(read_back[1].key, "k2");
        assert_eq!(read_back[1].entry_type, "investigation");
    }

    #[test]
    fn test_append_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.jsonl");

        let entry1 = MemoryEntry {
            key: "k1".into(),
            entry_type: "learned".into(),
            content: "c1".into(),
            source: "s".into(),
            tags: vec![],
            ts: 1,
            bead: "".into(),
        };

        let entry2 = MemoryEntry {
            key: "k2".into(),
            entry_type: "investigation".into(),
            content: "c2".into(),
            source: "s".into(),
            tags: vec![],
            ts: 2,
            bead: "".into(),
        };

        append_entry(&path, &entry1).unwrap();
        append_entry(&path, &entry2).unwrap();

        let entries = read_entries(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "k1");
        assert_eq!(entries[1].key, "k2");
    }

    #[test]
    fn test_knowledge_path() {
        let project = PathBuf::from("/home/user/project");
        let kp = knowledge_path(&project);
        assert_eq!(
            kp,
            PathBuf::from("/home/user/project/.beads/memory/knowledge.jsonl")
        );
    }

    #[test]
    fn test_archive_path() {
        let project = PathBuf::from("/home/user/project");
        let ap = archive_path(&project);
        assert_eq!(
            ap,
            PathBuf::from("/home/user/project/.beads/memory/knowledge.archive.jsonl")
        );
    }
}
