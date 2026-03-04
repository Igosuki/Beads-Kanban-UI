
## Beads-Kanban-UI-yii.4: Dolt server lifecycle manager (2026-02-27)

Created `server/src/backend/dolt_server.rs` with `DoltServerManager` (spawns `dolt sql-server` per project path with exponential back-off TCP readiness probing) and `DoltConnection` (wraps `mysql_async::Pool`). Added `mysql_async = "0.34"` dependency. The pre-commit hook runs `cargo clippy -- -D warnings` and requires the Next.js `out/` directory to exist (create temporarily before committing). Pre-existing clippy lints from the merged `dolt-backend-support` branch required fixing: `#[allow(dead_code)]` on `BeadsError`/`BeadsBackend` in `backend/mod.rs`, and `sort_by_key` in `memory.rs`.

## Beads-Kanban-UI-llv: Wire real JsonlBackend into BackendRegistry (2026-03-04)

Replaced the local stub `JsonlBackend` in `server/src/backend/detect.rs` with an import of `crate::backend::jsonl::JsonlBackend`. Removed the 38-line stub struct+impl block and three `#[allow(dead_code)]` suppressions that were only needed for the stub scaffolding. The `registry_creates_jsonl_backend_for_plain_project` test remains valid since `get_or_create()` only instantiates the backend (it does not call `read_beads`). All 129 tests pass; clippy pre-commit hook passes.
## 2026-03-04: Dolt backend fallback to JSONL

When `detect_backend()` returns `BackendType::Dolt` and Dolt is not yet implemented, `BackendRegistry::get_or_create()` now falls back to `JsonlBackend` if `.beads/issues.jsonl` exists, logging a `tracing::warn!`. Projects with only a dolt dir and no issues.jsonl still return a `BeadsError::Database` error.

## 2026-03-04: '+' button visibility in KanbanColumn

Changed the '+' button className in `src/components/kanban-column.tsx` from `text-zinc-500 hover:text-zinc-300` to `text-zinc-400 hover:text-zinc-200` for improved visibility on dark backgrounds.
