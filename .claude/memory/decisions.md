
## Beads-Kanban-UI-yii.4: Dolt server lifecycle manager (2026-02-27)

Created `server/src/backend/dolt_server.rs` with `DoltServerManager` (spawns `dolt sql-server` per project path with exponential back-off TCP readiness probing) and `DoltConnection` (wraps `mysql_async::Pool`). Added `mysql_async = "0.34"` dependency. The pre-commit hook runs `cargo clippy -- -D warnings` and requires the Next.js `out/` directory to exist (create temporarily before committing). Pre-existing clippy lints from the merged `dolt-backend-support` branch required fixing: `#[allow(dead_code)]` on `BeadsError`/`BeadsBackend` in `backend/mod.rs`, and `sort_by_key` in `memory.rs`.
