//! The record pipeline for project preferences — from stored JSON bytes back
//! to a validated record, and from fresh text to a validated new record —
//! plus the one read, a worktree's ordered list.

use saya_types::{
    PROJECT_PREFERENCE_VERSION, PreferenceId, PreferenceSource, ProjectPreference, WorktreeId,
};
use sha2::{Digest, Sha256};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{SqliteStateStore, StoreError};

/// Every preference stored for `worktree`, ordered by creation then id.
///
/// A row that fails to decode, or whose record disagrees with its own
/// columns, is skipped — never returned, and never an error for the others:
/// a listing must not be held hostage by one bad row, and a record whose
/// internal identity disagrees with its row is not this worktree's data.
pub(crate) async fn list_project_preferences(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
) -> Result<Vec<ProjectPreference>, StoreError> {
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, record_json FROM project_preferences WHERE worktree = ? ORDER BY created_unix_ms, id",
    )
    .bind(worktree.as_str())
    .fetch_all(pool)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    let mut out = Vec::with_capacity(rows.len());
    for (row_id, record_json) in rows {
        let Ok(record) = ProjectPreference::from_json_bytes(record_json.as_bytes()) else {
            continue;
        };
        if record.worktree.as_str() != worktree.as_str() || record.id.as_str() != row_id {
            continue;
        }
        out.push(record);
    }
    Ok(out)
}

/// Minted ids: a process-wide counter, so two adds in one process never mint
/// the same id.
static MINT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mints `pp-` + 32 lowercase hex: sha256 over nanos-since-epoch, the process
/// id, and the process-wide counter. No shared RNG and no time-only id.
pub(crate) fn mint_preference_id() -> PreferenceId {
    let counter = MINT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_string().as_bytes());
    hasher.update(process::id().to_be_bytes());
    hasher.update(counter.to_be_bytes());
    let hex: String = hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    PreferenceId::parse(&format!("pp-{hex}")).expect("a minted id matches the pp- shape")
}

/// Builds and validates the record for one add: `UserExplicit` provenance,
/// created = updated = `now_unix_ms.max(0)`, a freshly minted id.
/// Secret-shaped text is refused before anything is built.
pub(crate) fn build_record(
    worktree: &WorktreeId,
    text: &str,
    now_unix_ms: i64,
) -> Result<ProjectPreference, StoreError> {
    if crate::redact(text) != text {
        return Err(StoreError::Invalid);
    }
    let stamp = now_unix_ms.max(0);
    let record = ProjectPreference {
        version: PROJECT_PREFERENCE_VERSION,
        id: mint_preference_id(),
        worktree: worktree.clone(),
        text: text.to_owned(),
        source: PreferenceSource::UserExplicit,
        created_unix_ms: stamp,
        updated_unix_ms: stamp,
    };
    record.validate().map_err(|_| StoreError::Invalid)?;
    Ok(record)
}
