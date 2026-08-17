//! Insert/replace for the knowledge-items repository.
//!
//! The repository decides replace vs. append from the slot's cardinality, so
//! the caller never picks the wrong operation. A single-valued slot has a
//! stable id derived from the object and slot (the value does not take part),
//! so a re-write upserts the same row; the partial unique index is the
//! database-level backstop that refuses two single-valued rows even if a
//! future build or a direct insert bypassed the id scheme. A multi-valued
//! slot's id takes the value into account, so distinct values are distinct
//! rows and a re-file of the same value is an idempotent update; the declared
//! bound is checked before a new row is added.

use crate::contracts::admission;
use crate::contracts::store::now;
use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::binding::slot_matches_payload;
use crate::knowledge_items::keys::{knowledge_item_id, knowledge_item_id_value};
use crate::knowledge_items::records::MAX_KNOWLEDGE_ITEM_BYTES;
use crate::{SqliteStateStore, StoreError, redact};
use saya_types::{KnowledgeState, MAX_MULTI_SLOT_VALUES, SchemaFingerprint};

/// Insert or replace one knowledge item. A single-valued slot replaces the one
/// row it admits; a multi-valued slot appends a distinct value, refused past
/// its declared bound.
pub(crate) async fn insert_or_replace(
    store: &SqliteStateStore,
    request: &crate::knowledge_items::KnowledgeItemRequest,
) -> Result<(), KnowledgeStoreError> {
    let slot = &request.slot;
    if !slot_matches_payload(slot, &request.value) {
        return Err(KnowledgeStoreError::CardinalityMismatch);
    }
    let serialized = serde_json::to_string(&request.value).map_err(|_| StoreError::Invalid)?;
    if serialized.len() > MAX_KNOWLEDGE_ITEM_BYTES {
        return Err(StoreError::LimitExceeded.into());
    }
    // Same discipline as the claim payload: a credential or raw-SQL shape is
    // refused, not scrubbed-and-stored. `schema_binding_json` is a second
    // persisted channel, so it gets the same gate.
    if redact(&serialized) != serialized {
        return Err(StoreError::Invalid.into());
    }
    admission::check(&serialized)?;
    if redact(&request.schema_binding_json) != request.schema_binding_json {
        return Err(StoreError::Invalid.into());
    }
    admission::check(&request.schema_binding_json)?;

    let single = slot.cardinality().is_single();
    let id = if single {
        knowledge_item_id(&request.object, slot)
    } else {
        knowledge_item_id_value(&request.object, slot, &serialized)
    };
    let stamp = now();
    let mut tx = store
        .pool()
        .await
        .map_err(|_| StoreError::Unavailable)?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;

    if !single {
        // A new value appends; a re-file of an existing value is an idempotent
        // update that must not count toward the bound. Count only when the row
        // is genuinely new, so re-observing the same alias never refuses.
        let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_items WHERE id=?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        if existing == 0 {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM knowledge_items WHERE profile_id=? AND catalog=? AND schema=? AND object=? AND object_kind=? AND slot=? AND cardinality='multi'",
            )
            .bind(request.object.profile().as_str())
            .bind(request.object.catalog())
            .bind(request.object.schema())
            .bind(request.object.object())
            .bind(request.object.kind().as_str())
            .bind(slot.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
            if count >= MAX_MULTI_SLOT_VALUES as i64 {
                // Drop the transaction before returning; the bound refusal
                // wrote nothing, and leaving the tx to drop in-flight would
                // hold the write lock until the connection returned to the pool.
                tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
                return Err(KnowledgeStoreError::BoundExceeded);
            }
        }
    }

    sqlx::query("INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET value_json=excluded.value_json, source=excluded.source, state=excluded.state, schema_binding_json=excluded.schema_binding_json, fingerprint_version=excluded.fingerprint_version, updated_unix_ms=excluded.updated_unix_ms")
        .bind(&id)
        .bind(request.object.profile().as_str())
        .bind(request.object.catalog())
        .bind(request.object.schema())
        .bind(request.object.object())
        .bind(request.object.kind().as_str())
        .bind(slot.as_str())
        .bind(if single { "single" } else { "multi" })
        .bind(&serialized)
        .bind(request.source.as_str())
        .bind(request.state.as_str())
        .bind(&request.schema_binding_json)
        .bind(request.fingerprint.version() as i64)
        .bind(stamp)
        .bind(stamp)
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(())
}

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
