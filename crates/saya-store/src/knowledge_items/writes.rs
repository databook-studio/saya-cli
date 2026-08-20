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
use crate::contracts::now;
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
    // Blanking the row is not erasure on its own. In WAL mode the pre-update
    // page image lives in the `-wal` file, so the original text stays readable
    // on disk until a checkpoint folds the WAL back and `secure_delete` zeroes
    // the freed cell. Without this, `forget` honours the deletion promise in the
    // API and breaks it in the bytes — which is what `knowledge_security.rs`
    // scans for. TRUNCATE rather than PASSIVE so the WAL does not keep the copy.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(store.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
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

/// The serialised binding a forgotten row is reduced to: `SchemaBinding::Table`,
/// which names no column. The deletion promise (`docs/memory.md` §Deletion) is
/// that a forgotten fact's referenced columns are cleared, and a binding that
/// depends only on the table existing carries no column dependency. It is valid
/// `SchemaBinding` JSON, so a read that decodes a dismissed row's binding does
/// not fail — though no live path reads a dismissed row's binding (validity
/// returns `Invalid` for `Dismissed` before consulting it, and `confirm`
/// refuses a dismissed row with `Conflict` before revalidation).
const BLANKED_BINDING_JSON: &str = r#"{"type":"table"}"#;

/// Forget `id`: erase its content in the same transaction that marks it
/// `Dismissed`, then keep the row as a tombstone.
///
/// `value_json` is replaced with the serialised [`ClaimPayload::blanked`] of the
/// row's current value — the same variant with its free-text fields emptied, so
/// the secret-bearing channels (a description, a grain, an alias where a user or
/// the model could have pasted a credential) carry nothing. `schema_binding_json`
/// is replaced with [`BLANKED_BINDING_JSON`], which names no column. The `id`,
/// object identity, `slot`, `source`, `fingerprint_version`, and timestamps are
/// untouched, so "why did SAYA stop using that?" stays answerable and a
/// re-remember of the same fact lands on the same row (its id is value-independent
/// for a single-valued slot, and the row keeps its id for a multi-valued one) to
/// report *previously forgotten* rather than silently resurrecting.
///
/// One transaction: a crash cannot leave a `Dismissed` row that still holds its
/// content. The row is read inside the tx so the blanked value is derived from
/// what is actually stored, not from a caller-supplied echo; an unknown id is
/// `NotFound`. The blanked value is re-serialised through `ClaimPayload`'s serde
/// (the same form it was written under), so a later read decodes it — clearing
/// `value_json` to a non-`ClaimPayload` JSON like `null` would break every read
/// of the object, because `decode_item` deserialises the value of every row
/// including dismissed ones.
pub(crate) async fn forget_item(
    store: &SqliteStateStore,
    id: &str,
) -> Result<(), KnowledgeStoreError> {
    let stamp = now();
    let mut tx = store
        .pool()
        .await
        .map_err(|_| StoreError::Unavailable)?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let value_json: Option<String> =
        sqlx::query_scalar("SELECT value_json FROM knowledge_items WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    let Some(value_json) = value_json else {
        // An unknown id is a typed `NotFound`, not a silent no-op — the caller
        // asked to forget a fact that is not there.
        tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
        return Err(StoreError::NotFound.into());
    };
    // Derive the blanked payload from the stored value rather than trusting a
    // caller's claim about it. A row written by an incompatible build whose
    // value this build cannot decode cannot be safely blanked — fail closed
    // rather than write a `Dismissed` row whose value is not what we read.
    let payload: saya_types::ClaimPayload =
        serde_json::from_str(&value_json).map_err(|_| StoreError::Invalid)?;
    let blanked = serde_json::to_string(&payload.blanked()).map_err(|_| StoreError::Invalid)?;
    sqlx::query(
        "UPDATE knowledge_items SET value_json=?, schema_binding_json=?, state='dismissed', updated_unix_ms=? WHERE id=?",
    )
    .bind(&blanked)
    .bind(BLANKED_BINDING_JSON)
    .bind(stamp)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    // Blanking the row is not erasure on its own. In WAL mode the pre-update
    // page image lives in the `-wal` file, so the original text stays readable
    // on disk until a checkpoint folds the WAL back and `secure_delete` zeroes
    // the freed cell. Without this, `forget` honours the deletion promise in the
    // API and breaks it in the bytes — which is what `knowledge_security.rs`
    // scans for. TRUNCATE rather than PASSIVE so the WAL does not keep the copy.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(store.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(())
}
