use crate::StoreError;
use sqlx::{Sqlite, pool::PoolConnection};

/// Step 6 (Spec G, Chunk 5): drop the legacy `contract_*` tables. Phases D and
/// F moved every read and write to `knowledge_items`, so the four legacy tables
/// are dead weight. Nothing has shipped, so there is no data to preserve and no
/// compatibility window: drop. `user_preferences`, `schema_cache`, `audit_log`,
/// and `knowledge_items` are untouched (the knowledge path is the product now).
///
/// A new step, not an amendment: `user_version = 5` is a real state a
/// developer's unreleased database may hold, and a `CREATE TABLE IF NOT EXISTS`
/// step cannot remove a table in place. `IF EXISTS` makes the drop safe on every
/// path, including a fresh database that never created the tables.
pub(super) async fn step6(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    // Drop order: leaves before roots, so a foreign-key-enabled connection never
    // dangles a reference mid-drop. `contract_evidence` and `contract_events`
    // both reference `contract_claims`; `contract_claims` references
    // `contract_objects`.
    sqlx::query("DROP TABLE IF EXISTS contract_evidence")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("DROP TABLE IF EXISTS contract_events")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("DROP TABLE IF EXISTS contract_claims")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("DROP TABLE IF EXISTS contract_objects")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 6")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

/// Step 7 (run spine): the `runs` and `run_steps` tables — metadata only.
///
/// A run's goal, plan, and event journal live in its run directory; these
/// rows carry identity, status, timestamps, typed failure codes, capability
/// flags, and budget/usage figures. The usage columns are nullable: an
/// unreported figure is NULL — unknown, never zero. `budget_tokens_json` and
/// `usage_tokens_json` hold per-endpoint count maps keyed by validated
/// endpoint names; they are the only string-bearing columns, and the write
/// path refuses a credential-shaped key before any INSERT. The run dir is
/// derived from the run id, so no path column exists — the no-paths
/// discipline the store already holds.
///
/// A new step, not an amendment: `user_version = 6` is a real state a
/// developer's database may hold (step 6 dropped the legacy contract tables),
/// and a `CREATE TABLE IF NOT EXISTS` cannot be retrofitted into step 6
/// without rewriting a shipped ladder. Fresh and upgrading databases both
/// land here.
pub(super) async fn step7(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY, status TEXT NOT NULL, failure_code TEXT, cap_workspace_write INTEGER NOT NULL, cap_fetch INTEGER NOT NULL, cap_runner INTEGER NOT NULL, cap_scratch INTEGER NOT NULL, budget_wall_clock_ms INTEGER, budget_tokens_json TEXT, budget_turns INTEGER, budget_tool_calls INTEGER, budget_downloaded_bytes INTEGER, budget_workspace_bytes INTEGER, budget_workspace_files INTEGER, budget_process_count INTEGER, budget_process_time_ms INTEGER, usage_wall_clock_ms INTEGER, usage_tokens_json TEXT, usage_turns INTEGER, usage_tool_calls INTEGER, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS run_steps(run_id TEXT NOT NULL REFERENCES runs(id), step INTEGER NOT NULL, status TEXT NOT NULL, usage_wall_clock_ms INTEGER, usage_tokens_json TEXT, usage_turns INTEGER, usage_tool_calls INTEGER, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL, PRIMARY KEY (run_id, step))").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 7")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

/// Step 9 (M02): the `project_preferences` table — explicit, user-confirmed
/// project guidance scoped to one canonical worktree, in a table of its own.
/// It is not the typed `user_preferences` table (a setting, never prose) and
/// not typed database knowledge (`knowledge_items` is untouched); a preference
/// row carries no authority. `record_json` holds the whole validated record
/// and `text_bytes` the byte count the aggregate recall cap sums; the index
/// serves a worktree's list ordered by creation.
///
/// A new step, not an amendment: `user_version = 8` is a real state a
/// developer's database may hold, so the ladder arms `8 => step9` and every
/// earlier arm runs it too. An older binary opening this file is refused
/// (`VersionUnsupported`) — a downgrade must restore from a pre-upgrade
/// backup, never rewrite the rows.
pub(super) async fn step9(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS project_preferences(id TEXT PRIMARY KEY, worktree TEXT NOT NULL, text_bytes INTEGER NOT NULL, record_json TEXT NOT NULL, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE INDEX IF NOT EXISTS project_preferences_worktree ON project_preferences(worktree, created_unix_ms, id)")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 9")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}
