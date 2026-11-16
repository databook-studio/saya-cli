//! Mutations for the project-preferences repository that must enforce a
//! bound inside one transaction: add and edit.
//!
//! Both read a state (row count, aggregate sum, the row itself) and write a
//! decision from it, so their transactions must hold the write lock from the
//! start: sqlx's plain `begin()` is a DEFERRED transaction, and under WAL a
//! sibling commit between such a read and its write fails with a snapshot
//! conflict the busy timeout cannot wait out. `BEGIN IMMEDIATE` takes the
//! lock up front and makes the bound checks race-free. Every refusal rolls
//! back before returning: nothing is written, no in-flight transaction holds
//! the write lock.

use saya_types::{
    MAX_PREFERENCE_RECALL_BYTES, MAX_PROJECT_PREFERENCES, PreferenceId, ProjectPreference,
    WorktreeId,
};

use crate::project_preferences::reads::build_record;
use crate::{SqliteStateStore, StoreError};

/// Add one preference: count and aggregate checks and the insert share one
/// `BEGIN IMMEDIATE` transaction, so two racing writers cannot both pass the
/// checks. A minted id colliding with an existing row is the one retryable
/// outcome — rebuild with a fresh id and re-run the checks.
pub(crate) async fn add_project_preference(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
    text: &str,
    now_unix_ms: i64,
) -> Result<ProjectPreference, StoreError> {
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    for _ in 0..4 {
        let record = build_record(worktree, text, now_unix_ms)?;
        let serialized = serde_json::to_string(&record).map_err(|_| StoreError::Invalid)?;
        let text_bytes = record.text.len() as i64;
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let (count, sum): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(text_bytes), 0) FROM project_preferences WHERE worktree = ?",
        )
        .bind(worktree.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        if count >= MAX_PROJECT_PREFERENCES as i64
            || sum + text_bytes > MAX_PREFERENCE_RECALL_BYTES as i64
        {
            tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
            return Err(StoreError::LimitExceeded);
        }
        let inserted = sqlx::query("INSERT INTO project_preferences(id, worktree, text_bytes, record_json, created_unix_ms, updated_unix_ms) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING")
            .bind(record.id.as_str())
            .bind(worktree.as_str())
            .bind(text_bytes)
            .bind(&serialized)
            .bind(record.created_unix_ms)
            .bind(record.updated_unix_ms)
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        if inserted.rows_affected() == 0 {
            tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
            continue;
        }
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        store.secure_files()?;
        return Ok(record);
    }
    Err(StoreError::Conflict)
}

/// Replace one preference's text. The row is loaded by id AND worktree inside
/// the same transaction that updates it — an absent row and another
/// worktree's row are the same `NotFound` — and the aggregate recheck sums
/// only the other rows, so an edit can never push its worktree past the cap.
pub(crate) async fn edit_project_preference(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
    id: &PreferenceId,
    new_text: &str,
    now_unix_ms: i64,
) -> Result<ProjectPreference, StoreError> {
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT record_json FROM project_preferences WHERE id = ? AND worktree = ?")
            .bind(id.as_str())
            .bind(worktree.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    let Some((record_json,)) = row else {
        tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
        return Err(StoreError::NotFound);
    };
    let stored = ProjectPreference::from_json_bytes(record_json.as_bytes())
        .map_err(|_| StoreError::Invalid)?;
    if stored.id.as_str() != id.as_str() || stored.worktree.as_str() != worktree.as_str() {
        tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
        return Err(StoreError::Invalid);
    }
    let edited = stored
        .edited(new_text, now_unix_ms)
        .map_err(|_| StoreError::Invalid)?;
    if crate::redact(new_text) != new_text {
        tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
        return Err(StoreError::Invalid);
    }
    let others: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(text_bytes), 0) FROM project_preferences WHERE worktree = ? AND id <> ?",
    )
    .bind(worktree.as_str())
    .bind(id.as_str())
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if others + edited.text.len() as i64 > MAX_PREFERENCE_RECALL_BYTES as i64 {
        tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
        return Err(StoreError::LimitExceeded);
    }
    let serialized = serde_json::to_string(&edited).map_err(|_| StoreError::Invalid)?;
    sqlx::query(
        "UPDATE project_preferences SET text_bytes = ?, record_json = ?, updated_unix_ms = ? WHERE id = ? AND worktree = ?",
    )
    .bind(edited.text.len() as i64)
    .bind(&serialized)
    .bind(edited.updated_unix_ms)
    .bind(id.as_str())
    .bind(worktree.as_str())
    .execute(&mut *tx)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(edited)
}
