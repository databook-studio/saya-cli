//! Light row amendments: state updates, revalidation, and deletion.

use crate::contracts::{admission, now};
use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::records::MAX_SCHEMA_BINDING_BYTES;
use crate::{SqliteStateStore, StoreError, redact};
use saya_types::{KnowledgeState, SchemaFingerprint};

/// Update the state of an existing knowledge item.
pub(crate) async fn update_state(
    store: &SqliteStateStore,
    id: &str,
    state: KnowledgeState,
) -> Result<(), KnowledgeStoreError> {
    let stamp = now();
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("UPDATE knowledge_items SET state=?, updated_unix_ms=? WHERE id=?")
        .bind(state.as_str())
        .bind(stamp)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(())
}

/// Revalidate an item, updating its schema binding JSON, fingerprint version,
/// and transitioning its state to `Active`.
pub(crate) async fn revalidate_item(
    store: &SqliteStateStore,
    id: &str,
    fingerprint: SchemaFingerprint,
    schema_binding_json: String,
) -> Result<(), KnowledgeStoreError> {
    if schema_binding_json.len() > MAX_SCHEMA_BINDING_BYTES {
        return Err(StoreError::LimitExceeded.into());
    }
    if redact(&schema_binding_json) != schema_binding_json {
        return Err(StoreError::Invalid.into());
    }
    admission::check(&schema_binding_json)?;
    let stamp = now();
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query(
        "UPDATE knowledge_items SET state='active', schema_binding_json=?, fingerprint_version=?, updated_unix_ms=? WHERE id=?",
    )
    .bind(&schema_binding_json)
    .bind(fingerprint.version() as i64)
    .bind(stamp)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(())
}

/// Delete a knowledge item by its unique ID.
pub(crate) async fn delete_item(
    store: &SqliteStateStore,
    id: &str,
) -> Result<(), KnowledgeStoreError> {
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("DELETE FROM knowledge_items WHERE id=?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(())
}
