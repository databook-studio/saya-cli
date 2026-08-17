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
/// `(object, slot, value). If it exists, **nothing is written** and the
/// existing item's state is returned (`Duplicate`). So re-remembering a
/// forgotten fact reports `Duplicate { Dismissed }` rather than silently
/// reviving it, and re-remembering an active fact reports `Duplicate { Active }`
/// with the same id. A genuinely new fact is `put_knowledge_item`'d as `Active`
/// (`UserExplicit`) and its `ki-…` id returned (`Stored`).
///
/// **One exception (Open Question 2):** a directive claim re-stated with the
/// same value but a *new* reason is a **revision**, not a duplicate. The
/// single-valued slot's value is unchanged (the dedup key does not move), but
/// the reason is the one field this write path can refine, and a user who just
/// explained themselves must not see nothing happen. So when the existing
/// item is not a forgotten tombstone and the new payload differs from it only
/// in `reason`, the row is rewritten with the new reason and the outcome is
/// `Stored` (a revision wrote). A genuinely identical re-remember — same
/// value *and* same reason, or no reason on either side — is still a no-op
/// `Duplicate`. A forgotten tombstone is still never resurrected: a re-remember
/// of a dismissed item reports `Duplicate { Dismissed }` even with a reason.
///
/// The lookup keys on the **row id** the put would land on, not on the decoded
/// value. `forget` blanks a tombstone's value (the deletion promise), so a
/// value comparison would miss a forgotten multi-valued item and the re-remember
/// would silently resurrect it; the tombstone keeps its id, so an id lookup
/// still finds it. This is the half of the deletion guarantee that makes erasure
/// safe: the row is content-free *and* still recognised as the same fact.
pub(crate) async fn remember(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: &ClaimPayload,
    fingerprint: SchemaFingerprint,
) -> Result<RememberOutcome, ContractOpError> {
    let slot = slot_for_payload(payload).ok_or(ContractOpError::Invalid)?;
    // A duplicate is an existing item at the id the put would land on. Looking
    // up by id before the write (rather than upserting and reading back) keeps
    // the no-write semantics: a forgotten tombstone stays forgotten, an active
    // claim is not overwritten — unless the new payload revises the reason
    // (see the module / outcome docs).
    if let Some(existing) = find_existing(store, object, &slot, payload).await? {
        // A forgotten tombstone is never resurrected: report the dismissed
        // duplicate even if a reason is now supplied.
        if existing.state != KnowledgeState::Dismissed
            && is_reason_revision(payload, &existing.value)
        {
            // Same value, new reason: revise the row in place. The value is
            // unchanged so the binding and fingerprint derivation are too; only
            // the payload (carrying the reason) is rewritten.
            let binding = SchemaBinding::derive(&slot, payload).ok_or(ContractOpError::Invalid)?;
            let binding_json =
                serde_json::to_string(&binding).map_err(|_| ContractOpError::Unavailable)?;
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
            let id = item_id_for(object, &slot, payload)?;
            return Ok(RememberOutcome::Stored { id });
        }
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
    let id = item_id_for(object, &slot, payload)?;
    Ok(RememberOutcome::Stored { id })
}

/// True when `new` carries the same directive value as `old` but a *new,
/// explicitly-stated* reason — the one case `remember` revises instead of
/// dedups. The comparison is on the value-defining fields (column, role,
/// description) plus the reason: a difference in anything but the reason is a
/// genuine value change (which cannot land on the same single-valued row
/// anyway, and is a new fact for a multi-valued slot), and a difference in the
/// reason alone is a revision. Only a *present* new reason revises: re-stating
/// the same value with no reason does not erase an existing one — silence is
/// not "drop the reason," and the spec's case is a user who states a *new*
/// reason. Non-directive kinds never carry a reason, so they never revise —
/// an identical re-remember of a description is a plain duplicate. The `old`
/// payload is the stored one (possibly blanked for a tombstone, but a tombstone
/// is excluded by the caller's `Dismissed` guard).
fn is_reason_revision(new: &ClaimPayload, old: &ClaimPayload) -> bool {
    use saya_types::ClaimPayload as P;
    match (new, old) {
        (
            P::DefaultTimeColumn {
                column: c1,
                reason: r1,
                ..
            },
            P::DefaultTimeColumn {
                column: c2,
                reason: r2,
                ..
            },
        ) => c1 == c2 && r1.is_some() && r1 != r2,
        (
            P::TableGrain {
                description: d1,
                reason: r1,
                ..
            },
            P::TableGrain {
                description: d2,
                reason: r2,
                ..
            },
        ) => d1 == d2 && r1.is_some() && r1 != r2,
        (
            P::ColumnRole {
                column: c1,
                role: rl1,
                reason: rr1,
                ..
            },
            P::ColumnRole {
                column: c2,
                role: rl2,
                reason: rr2,
                ..
            },
        ) => c1 == c2 && rl1 == rl2 && rr1.is_some() && rr1 != rr2,
        // Different kinds, or a non-directive kind, never carry a reason to
        // revise — not a revision.
        _ => false,
    }
}

/// The existing item at the id a put of `payload` under `slot` on `object` would
/// land on, if any. The id is value-independent for a single-valued slot
/// (`(object, slot)`) and takes the value into account for a multi-valued one
/// (`(object, slot, value)`), so this is the same dedup key the store's
/// `ON CONFLICT(id)` uses. Keying on the id (not the decoded value) is what
/// makes a forgotten tombstone — whose value `forget` has blanked — still
/// recognised as the duplicate of a re-proposed same-value fact.
async fn find_existing(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    slot: &KnowledgeSlot,
    payload: &ClaimPayload,
) -> Result<Option<KnowledgeItem>, ContractOpError> {
    let serialized = serde_json::to_string(payload).map_err(|_| ContractOpError::Unavailable)?;
    let id = saya_store::knowledge_item_id_for(object, slot, &serialized);
    Ok(store.get_knowledge_item(&id).await?)
}

/// The `ki-…` id of the item a put of `payload` under `slot` on `object` lands
/// on — the same id [`find_existing`] looked up, derived the same way the store
/// derives it on write. Called after a successful `put_knowledge_item`, so the
/// row exists at exactly this id; the id is derived, not read back, because the
/// store just wrote it under this id and a read would only echo it.
fn item_id_for(
    object: &DatabaseObjectRef,
    slot: &KnowledgeSlot,
    payload: &ClaimPayload,
) -> Result<ClaimId, ContractOpError> {
    let serialized = serde_json::to_string(payload).map_err(|_| ContractOpError::Unavailable)?;
    let id = saya_store::knowledge_item_id_for(object, slot, &serialized);
    ClaimId::parse(&id).map_err(|_| ContractOpError::Invalid)
}
