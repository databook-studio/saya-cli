//! Physical cleanup shared by the write paths: fold the WAL back and zero
//! the freed bytes, so a blanked or replaced row's text does not stay
//! readable on disk.

use crate::knowledge_items::KnowledgeStoreError;
use crate::{SqliteStateStore, StoreError};

/// Checkpoint the WAL (TRUNCATE) and secure the database files. Called after
/// every content-erasing write; see the insert and forget paths.
pub(super) async fn cleanup_files(store: &SqliteStateStore) -> Result<(), KnowledgeStoreError> {
    if store.consume_cleanup_failure_for_tests() {
        return Err(StoreError::Unavailable.into());
    }
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(store.pool().await.map_err(|_| StoreError::Unavailable)?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(())
}
