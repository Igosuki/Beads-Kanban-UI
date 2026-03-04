# Dolt Backend Support Design

## Summary

Add support for the Dolt database backend alongside the existing JSONL backend. The Kanban UI will auto-detect which backend a project uses and route reads/writes accordingly. Feature parity with JSONL — no new Dolt-exclusive UI features in this phase.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Read path | Direct MySQL via `mysql_async` | Single SQL query with JOINs replaces JSONL parsing + 4 post-processing passes |
| Write path | `bd` CLI (unchanged) | Preserves bd's business logic: hooks, event trail, sync, validation |
| Embedded mode | Auto-start `dolt sql-server` on ephemeral port | Good UX — no manual server management needed |
| Change detection | Poll `dolt_hashof_table()` every 2-3s | Lightweight single-row query, reliable, works in all modes |
| Scope | Feature parity only | Dolt-exclusive features (history, diff, blame) deferred to follow-up |

## Architecture

```
GET /api/beads?path=...
     │
     ▼
detect_backend(project_path)
     │
     ├── no-db / no .beads/dolt/ ──► JsonlBackend (existing code, unchanged)
     │
     └── .beads/dolt/ exists ──► DoltBackend
         ├── READ:  mysql_async connection pool
         │          SELECT + JOIN issues/comments/dependencies
         ├── WRITE: bd CLI (bd update, bd close, bd comments add)
         └── WATCH: poll dolt_hashof_table() every 2-3s, emit SSE on change
```

### Backend Detection (per project, cached)

1. Check `.beads/config.yaml` for `no-db: true` → JSONL
2. Check if `.beads/dolt/` directory exists → Dolt
3. Fallback → JSONL

### Dolt Server Lifecycle

For embedded mode (no external `dolt sql-server` running):

1. On first request to a Dolt-backed project, check if `bd dolt test` succeeds
2. If not, run `dolt sql-server` in `.beads/dolt/{db_name}/` on an ephemeral port
3. Store port in a connection registry keyed by project path
4. Connection pool via `mysql_async` with configurable pool size
5. Graceful shutdown: stop spawned servers when Rust backend exits

For server mode (user runs `bd dolt start` or external server):

1. Read connection details from `bd dolt show` (or `.beads/metadata.json`)
2. Connect to existing server (host:port from config)

### SQL Read Query

Replace the current 4-pass JSONL processing with a single SQL approach:

```sql
-- Main issues query
SELECT i.id, i.title, i.description, i.status, i.priority,
       i.issue_type, i.owner, i.created_at, i.created_by,
       i.updated_at, i.closed_at, i.close_reason, i.design
FROM issues i
WHERE i.status != 'tombstone'
ORDER BY i.priority, i.created_at;

-- Comments (separate query, grouped)
SELECT c.id, c.issue_id, c.author, c.text, c.created_at
FROM comments c
ORDER BY c.issue_id, c.created_at;

-- Dependencies (separate query)
SELECT d.issue_id, d.depends_on_id, d.type, d.created_at, d.created_by
FROM dependencies d;
```

Post-process in Rust: merge comments and dependencies into Bead structs, build parent-child relationships from `parent-child` type dependencies and ID pattern inference (same logic as current JSONL path).

### Change Detection (SSE Watcher)

```rust
// Polling loop for Dolt change detection
loop {
    let new_hash = query("SELECT dolt_hashof_table('issues')").await;
    let comments_hash = query("SELECT dolt_hashof_table('comments')").await;
    let deps_hash = query("SELECT dolt_hashof_table('dependencies')").await;

    if new_hash != last_issues_hash
        || comments_hash != last_comments_hash
        || deps_hash != last_deps_hash
    {
        emit_sse_event("modified");
        last_issues_hash = new_hash;
        last_comments_hash = comments_hash;
        last_deps_hash = deps_hash;
    }
    sleep(Duration::from_secs(2)).await;
}
```

### Backend Trait

```rust
#[async_trait]
pub trait BeadsBackend: Send + Sync {
    async fn read_beads(&self, project_path: &Path) -> Result<Vec<Bead>, BeadsError>;
    async fn add_comment(&self, project_path: &Path, bead_id: &str, text: &str, author: &str) -> Result<Bead, BeadsError>;
    fn watch(&self, project_path: &Path) -> Pin<Box<dyn Stream<Item = FileChangeEvent> + Send>>;
}
```

Two implementations:
- `JsonlBackend` — wraps existing code from `beads.rs` and `watch.rs`
- `DoltBackend` — MySQL reads + `bd` CLI writes + hash polling

### Frontend Changes

Minimal. The API response shape stays identical (`{ "beads": Bead[] }`). The frontend doesn't know or care which backend is used. The only potential change is the `add_comment` endpoint which currently writes JSONL directly — for Dolt projects it will use `bd comments add` instead.

## New Dependencies

- `mysql_async` — async MySQL client for Rust (Dolt is MySQL-compatible)
- `async-trait` — for the `BeadsBackend` trait

## File Changes

| File | Change |
|------|--------|
| `server/Cargo.toml` | Add `mysql_async`, `async-trait` |
| `server/src/backend/mod.rs` | New: `BeadsBackend` trait definition |
| `server/src/backend/jsonl.rs` | New: Extract existing JSONL logic into trait impl |
| `server/src/backend/dolt.rs` | New: Dolt MySQL backend implementation |
| `server/src/backend/detect.rs` | New: Backend detection and caching |
| `server/src/backend/dolt_server.rs` | New: Dolt server lifecycle management |
| `server/src/routes/beads.rs` | Refactor to use `BeadsBackend` trait |
| `server/src/routes/watch.rs` | Refactor to support both file watching and hash polling |
| `server/src/main.rs` | Initialize backend registry, pass to routes |

## Testing Strategy

- Unit tests for backend detection logic (mock config.yaml, mock .beads/dolt/)
- Unit tests for SQL result → Bead struct mapping
- Integration test: spin up `dolt sql-server` in tempdir, verify full read/write cycle
- Existing JSONL tests remain unchanged (regression safety)
