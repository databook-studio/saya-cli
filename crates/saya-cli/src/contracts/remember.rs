//! The `remember` write path: writes a `knowledge_items` row so a remembered
//! fact is the same row `show`, `queue` and recall read — no split brain with
//! the legacy `contract_claims` table the old `propose_claim` path wrote. Split
//! from [`super::review`] by concern: this module *creates* an item; the
//! confirm/reject/forget wrappers in `review` *mutate* an existing one.
//!
//! The slot is derived from the payload via [`super::args::slot_for_payload`]
//! (the one pairing the ingest path uses); the [`SchemaBinding`] is derived
//! from `(slot, payload)`; the fingerprint is the caller's — the real
//! `of_table` digest against a cached table, or the `unobserved` sentinel when
//! there is no schema to fingerprint against. The structural dependency a later
//! drift checks is the binding, so the column snapshots the legacy
//! `ProposeClaim` carried are not computed here.

use super::op_error::ContractOpError;
use crate::contracts::args::slot_for_payload;
use saya_store::{KnowledgeItem, KnowledgeItemStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, DatabaseObjectRef, KnowledgeSlot, KnowledgeState,
    SchemaBinding, SchemaFingerprint,
};

/// The result of remembering a fact: either a fresh row was stored, or the
/// `(object, slot[, value])` already had an item and the remember wrote nothing
/// — the existing item's state is reported so a duplicate of a forgotten
/// claim reads as forgotten, not as a silent success. Mirrors the legacy
/// `ProposeOutcome::Stored` / `Duplicate` distinction the command layer
/// rendered, so `remember` keeps the same observable behaviour after the
/// write-path migration onto `knowledge_items`.
///
/// `id` is the `ki-…` id of the row in either arm — the stored row for
/// `Stored`, the pre-existing row for `Duplicate` — so the command layer
/// renders one id consistently across text, JSON and NDJSON.
#[derive(Debug)]
pub(crate) enum RememberOutcome {
    Stored { id: ClaimId },
    Duplicate { id: ClaimId, state: KnowledgeState },
}

/// Remembers a fact: writes a `knowledge_items` row. See the module docs for
/// the slot/binding/fingerprint derivation.
///
/// Dedup mirrors the legacy `propose_claim` `Duplicate` outcome: before
/// writing, the item the `(object, slot[, value])` would land on is looked
/// up — a single-valued slot keys on `(object, slot)`, a multi-valued one on
/// `(object, slot, value)`. If it exists, **nothing is written** and the
/// existing item's state is returned (`Duplicate`). So re-remembering a
/// forgotten fact reports `Duplicate { Dismissed }` rather than silently
/// reviving it, and re-remembering an active fact reports `Duplicate { Active }`
/// with the same id. A genuinely new fact is `put_knowledge_item`'d as `Active`
/// (`UserExplicit`) and its `ki-…` id returned (`Stored`).
pub(crate) async fn remember(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: &ClaimPayload,
    fingerprint: SchemaFingerprint,
) -> Result<RememberOutcome, ContractOpError> {
    let slot = slot_for_payload(payload).ok_or(ContractOpError::Invalid)?;
    // A duplicate is an existing item at the (object, slot) — and for a
    // multi-valued slot, the same value. Looking up before the write (rather
    // than upserting and reading back) keeps the no-write semantics: a
    // forgotten tombstone stays forgotten, an active claim is not overwritten.
    if let Some(existing) = find_existing(store, object, &slot, payload).await? {
        let id = ClaimId::parse(&existing.id).map_err(|_| ContractOpError::Invalid)?;
        return Ok(RememberOutcome::Duplicate {
            id,
            state: existing.state,
        });
    }
    let binding = SchemaBinding::derive(&slot, payload).ok_or(ContractOpError::Invalid)?;
    let binding_json = serde_json::to_string(&binding).map_err(|_| ContractOpError::Unavailable)?;
    store
        .put_knowledge_item(saya_store::KnowledgeItemRequest {
            object: object.clone(),
            slot: slot.clone(),
            value: payload.clone(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: binding_json,
            fingerprint,
        })
        .await?;
    let id = item_id_for(store, object, &slot, payload).await?;
    Ok(RememberOutcome::Stored { id })
}

/// The existing item at `(object, slot[, value])`, if any. A single-valued slot
/// keys on `(object, slot)` alone (the value does not take part in the id), so
/// any existing item for the slot is a duplicate; a multi-valued slot keys on
/// the value too, so only the same value is a duplicate — a second alias for the
/// same object is a new row, not a duplicate of the first.
async fn find_existing(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    slot: &KnowledgeSlot,
    payload: &ClaimPayload,
) -> Result<Option<KnowledgeItem>, ContractOpError> {
    let items = store.knowledge_for_object(object).await?;
    Ok(items.into_iter().find(|item| {
        &item.slot == slot && (slot.cardinality().is_single() || item.value == *payload)
    }))
}

/// The `ki-…` id of the item at `(object, slot[, value])` after a put. The store
/// derives the id from `(object, slot)` for a single-valued slot and
/// `(object, slot, value)` for a multi-valued one, so there is exactly one
/// match; a `None` here means the put did not land, which is a store fault the
/// caller surfaces as `Unavailable`.
async fn item_id_for(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    slot: &KnowledgeSlot,
    payload: &ClaimPayload,
) -> Result<ClaimId, ContractOpError> {
    let item = store
        .knowledge_for_object(object)
        .await?
        .into_iter()
        .find(|item| {
            &item.slot == slot && (slot.cardinality().is_single() || item.value == *payload)
        })
        .ok_or(ContractOpError::Unavailable)?;
    ClaimId::parse(&item.id).map_err(|_| ContractOpError::Invalid)
}
