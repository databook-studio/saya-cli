//! Transactional batch insert for imported knowledge items.
//!
//! `contracts import` (ADR 0006) applies a validated set of claims from a team
//! file. The batch is one transaction: every item lands `Pending` or the whole
//! batch rolls back, an item already on file is skipped rather than re-written,
//! and a single-valued slot held by a different value — or a row the user
//! forgot — is reported as a conflict instead of being overwritten. The local
//! review decision outranks the file. Ids reuse the same derivation as the
//! single-item write path, so a batched import and a later single put resolve
//! to the same rows.

use crate::contracts::{admission, now};
use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::binding::slot_matches_payload;
use crate::knowledge_items::keys::{knowledge_item_id, knowledge_item_id_value};
use crate::knowledge_items::records::{
    CleanupState, MAX_KNOWLEDGE_ITEM_BYTES, MAX_SCHEMA_BINDING_BYTES,
};
use crate::{SqliteStateStore, StoreError, redact};
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectRef, KnowledgeSlot, KnowledgeState,
    MAX_MULTI_SLOT_VALUES, SchemaFingerprint,
};
use sqlx::SqliteConnection;

/// The largest batch [`apply_pending_batch`] accepts. A larger batch is
/// refused before any transaction is opened, so an oversized import writes
/// nothing at all.
pub const MAX_PENDING_BATCH_ITEMS: usize = 500;

/// One item offered to the batch — the single-item write request minus the
/// state: a batch import always lands `Pending`, so there is nothing for the
/// caller to set and no way to write an `Active` row through the batch.
#[derive(Debug, Clone)]
pub struct NewKnowledgeItem {
    pub object: DatabaseObjectRef,
    pub slot: KnowledgeSlot,
    pub value: ClaimPayload,
    pub source: ClaimOrigin,
    /// Pre-serialised schema binding, opaque to the store. Persisted beside
    /// the fingerprint version exactly as the single-item path does.
    pub schema_binding_json: String,
    pub fingerprint: SchemaFingerprint,
}

/// What became of one item of an applied batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchItemOutcome {
    /// Written as a new `Pending` row; `id` is the row it landed under — the
    /// same id a later single-item put of the same fact resolves to.
    Inserted { id: String },
    /// An identical item is already on file (`Pending` or `Active`); nothing
    /// was written.
    Skipped,
    /// The slot is already held by a row with a different value, or by a row
    /// the user forgot. The existing row is named; it was not touched.
    Conflict { existing_id: String },
}

/// The per-item results of an applied batch, aligned with the input order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchOutcome {
    pub items: Vec<BatchItemOutcome>,
}

/// Apply a batch of imported claims in one transaction. A conflict is an
/// expected outcome, not an error; any storage failure rolls everything back.
pub(crate) async fn apply_pending_batch(
    store: &SqliteStateStore,
    items: &[NewKnowledgeItem],
) -> Result<BatchOutcome, KnowledgeStoreError> {
    if items.len() > MAX_PENDING_BATCH_ITEMS {
        return Err(KnowledgeStoreError::BoundExceeded);
    }
    let prepared: Vec<PreparedItem> = items.iter().map(prepare).collect::<Result<_, _>>()?;
    let mut tx = store
        .pool()
        .await
        .map_err(|_| StoreError::Unavailable)?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let mut results = Vec::with_capacity(prepared.len());
    for item in &prepared {
        match apply_one(&mut tx, item).await {
            Ok(outcome) => results.push(outcome),
            Err(error) => {
                tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
                return Err(error);
            }
        }
    }
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    // The batch inserts new rows only — nothing is overwritten, so unlike the
    // replace path no stale page image needs a checkpoint; tightening the file
    // permissions is the part that still applies.
    store.secure_files()?;
    Ok(BatchOutcome { items: results })
}

/// An item with its serialised value and derived id, ready to bind.
struct PreparedItem {
    object: DatabaseObjectRef,
    slot: KnowledgeSlot,
    source: ClaimOrigin,
    schema_binding_json: String,
    fingerprint: SchemaFingerprint,
    id: String,
    serialized: String,
    single: bool,
}

/// Validate one item up front, before the transaction opens: the same payload
/// discipline as the single-item path, then the id it would land under.
fn prepare(request: &NewKnowledgeItem) -> Result<PreparedItem, KnowledgeStoreError> {
    if !slot_matches_payload(&request.slot, &request.value) {
        return Err(KnowledgeStoreError::CardinalityMismatch);
    }
    let serialized = serde_json::to_string(&request.value).map_err(|_| StoreError::Invalid)?;
    if serialized.len() > MAX_KNOWLEDGE_ITEM_BYTES {
        return Err(StoreError::LimitExceeded.into());
    }
    if redact(&serialized) != serialized {
        return Err(StoreError::Invalid.into());
    }
    admission::check(&serialized)?;
    if redact(&request.schema_binding_json) != request.schema_binding_json {
        return Err(StoreError::Invalid.into());
    }
    if request.schema_binding_json.len() > MAX_SCHEMA_BINDING_BYTES {
        return Err(StoreError::LimitExceeded.into());
    }
    admission::check(&request.schema_binding_json)?;
    let single = request.slot.cardinality().is_single();
    let id = if single {
        knowledge_item_id(&request.object, &request.slot)
    } else {
        knowledge_item_id_value(&request.object, &request.slot, &serialized)
    };
    Ok(PreparedItem {
        object: request.object.clone(),
        slot: request.slot.clone(),
        source: request.source,
        schema_binding_json: request.schema_binding_json.clone(),
        fingerprint: request.fingerprint.clone(),
        id,
        serialized,
        single,
    })
}

/// Classify one item against the store inside the batch's transaction: the
/// row it resolves to decides skip, conflict, or insert.
async fn apply_one(
    connection: &mut SqliteConnection,
    item: &PreparedItem,
) -> Result<BatchItemOutcome, KnowledgeStoreError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT value_json, state FROM knowledge_items WHERE id=?")
            .bind(&item.id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    let Some((value_json, state)) = row else {
        insert_new(connection, item).await?;
        return Ok(BatchItemOutcome::Inserted {
            id: item.id.clone(),
        });
    };
    if state == KnowledgeState::Dismissed.as_str() {
        // The user forgot this fact. Never resurrect it and never overwrite
        // the tombstone; naming the row lets the importer report the item.
        return Ok(BatchItemOutcome::Conflict {
            existing_id: item.id.clone(),
        });
    }
    if !item.single || value_json == item.serialized {
        // A multi-valued slot's id takes the value into account, so the same
        // id is the same value; a single-valued slot storing the same
        // serialisation is the same fact. Both are the idempotent skip.
        return Ok(BatchItemOutcome::Skipped);
    }
    Ok(BatchItemOutcome::Conflict {
        existing_id: item.id.clone(),
    })
}

/// Insert one new row, always `Pending`, after the multi-valued slot's
/// declared bound is checked. A plain insert: the batch never updates an
/// existing row, so a concurrent writer taking the same id fails closed
/// instead of being overwritten.
async fn insert_new(
    connection: &mut SqliteConnection,
    item: &PreparedItem,
) -> Result<(), KnowledgeStoreError> {
    if !item.single {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_items WHERE profile_id=? AND catalog=? AND schema=? AND object=? AND object_kind=? AND slot=? AND cardinality='multi'")
            .bind(item.object.profile().as_str())
            .bind(item.object.catalog())
            .bind(item.object.schema())
            .bind(item.object.object())
            .bind(item.object.kind().as_str())
            .bind(item.slot.as_str())
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        if count >= MAX_MULTI_SLOT_VALUES as i64 {
            return Err(KnowledgeStoreError::BoundExceeded);
        }
    }
    let stamp = now();
    sqlx::query("INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, cleanup_state, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&item.id)
        .bind(item.object.profile().as_str())
        .bind(item.object.catalog())
        .bind(item.object.schema())
        .bind(item.object.object())
        .bind(item.object.kind().as_str())
        .bind(item.slot.as_str())
        .bind(if item.single { "single" } else { "multi" })
        .bind(&item.serialized)
        .bind(item.source.as_str())
        .bind(KnowledgeState::Pending.as_str())
        .bind(&item.schema_binding_json)
        .bind(CleanupState::Complete.as_str())
        .bind(item.fingerprint.version() as i64)
        .bind(stamp)
        .bind(stamp)
        .execute(&mut *connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}
