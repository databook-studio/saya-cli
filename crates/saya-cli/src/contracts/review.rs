//! Review operations: thin typed wrappers over [`KnowledgeItemStore`]. No
//! policy of their own beyond revalidating a confirm against the live schema —
//! the store enforces cardinality; this layer enforces "a confirm must not
//! revive a fact whose dependency is gone".
//!
//! Chunk 3 moved these onto `knowledge_items`: confirm/reject/forget mutate an
//! item's [`KnowledgeState`], show reads `knowledge_for_object`. The receipt
//! and list/show stanzas print `ki-…` ids, so the decision ops and the prefix
//! resolver all key on those (see [`super::decide`]).
//!
//! `confirm`/`reject` return the shared render carrier [`ContractClaim`] (built
//! from the post-mutation item) so the command layer that reads `claim.id` and
//! `claim.status` keeps compiling without reaching into `knowledge_items`
//! itself — the CLI presentation migration is a later chunk. The carrier carries
//! no fingerprint or columns, so returning it leaks nothing the store does not.

use super::op_error::ContractOpError;
use super::view::ContractClaim;

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use saya_store::{KnowledgeItem, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    BindingValidity, ClaimId, DatabaseObjectRef, KnowledgeState, SchemaBinding, SchemaFingerprint,
    Table,
};

/// Confirms `id`: a `Pending` candidate becomes `Active`, and an `Active` fact
/// is re-verified against the cached schema. Both paths revalidate when a
/// schema is known — confirming must never set `Active` on a fact whose
/// structural dependency is gone, because read-time validity is computed from
/// the binding, so a gone column would read `Invalid` the moment the confirm
/// landed (the revival bug). The refusal names the obstacle:
/// [`ContractOpError::ColumnGone`] when a bound column is gone, [`ObjectGone`]
/// when the table is, [`SchemaUnavailable`] when an existing `Active` fact
/// cannot be re-verified because no schema is cached.
///
/// A `Pending` candidate with no cached schema is confirmed without one: it is
/// the user *introducing* a fact (the user is the source), and it reads
/// `SchemaUnavailable` — honest, not a false `Current`. An `Active` fact with
/// no schema is *refused*: re-confirming an existing belief is a re-verification,
/// and there is nothing to verify against. That asymmetry is the D-3
/// translation of the legacy `Candidate`-vs-`Stale` distinction (a status-only
/// flip needed no schema; a stale claim had to be revalidated) — both preserved
/// by their tests.
///
/// A `Dismissed` item is withdrawn and not revivable; confirm refuses with
/// [`ContractOpError::Conflict`], matching the legacy legal-transition refusal.
pub(crate) async fn confirm(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<ContractClaim, ContractOpError> {
    let item = store
        .get_knowledge_item(id.as_str())
        .await?
        .ok_or(ContractOpError::NotFound)?;
    if matches!(item.state, KnowledgeState::Dismissed) {
        return Err(ContractOpError::Conflict);
    }
    let availability = schema_availability_for(store, item.object.profile().as_str()).await;
    // An empty cached tree (no `databases` — the no-op sentinel a fresh store
    // writes) carries no real schema information, so it is "no usable schema",
    // not "every object is gone". Reading it as `ObjectGone` is the same class
    // of bug as review item #29: an empty cache is absence of evidence, not
    // evidence the object is gone. `usable_table_schema` returns `None` for
    // `Missing`, `Unavailable`, *and* an empty `Available` tree, so all three
    // route to the no-schema arm; a populated tree that lacks the object's
    // table still reaches the `Some` arm and reads `ObjectGone` there.
    match usable_table_schema(&availability) {
        None => match item.state {
            // A candidate is the user asserting a new fact; no schema is fine —
            // it reads `SchemaUnavailable`, never a false `Current`.
            KnowledgeState::Pending => {
                store
                    .update_knowledge_item_state(id.as_str(), KnowledgeState::Active)
                    .await?;
            }
            // An Active fact is a re-verification; refuse without a schema to
            // check it against rather than rubber-stamp a belief we cannot vet.
            _ => return Err(ContractOpError::SchemaUnavailable),
        },
        Some(schema) => {
            let live_table = find_table(&item.object, schema).cloned();
            let live_table = live_table.ok_or(ContractOpError::ObjectGone)?;
            let binding = binding_to_validate(&item)?;
            // Name the actual obstacle before refusing generically. A `Table`
            // binding validates whenever the table exists, so `Invalid` here is
            // a `Column` binding whose column is gone or lost its semantic type.
            if binding.validate(&live_table) == BindingValidity::Invalid {
                return Err(ContractOpError::ColumnGone);
            }
            let fresh_binding =
                serde_json::to_string(&binding).map_err(|_| ContractOpError::Unavailable)?;
            let fingerprint = SchemaFingerprint::of_table(item.object.kind(), &live_table);
            store
                .revalidate_knowledge_item(id.as_str(), fingerprint, fresh_binding)
                .await?;
        }
    }
    // Re-read so the carrier reflects the post-mutation state the store now
    // holds — `revalidate`/`update_state` set `Active`, and the carrier's
    // `status` is derived from `state`.
    let item = store
        .get_knowledge_item(id.as_str())
        .await?
        .ok_or(ContractOpError::NotFound)?;
    ContractClaim::from_knowledge_item(&item).ok_or(ContractOpError::Unavailable)
}

/// Rejects `id`: a `Pending` candidate is moved to `Dismissed`. An `Active` or
/// `Dismissed` item refuses with [`ContractOpError::Conflict`] — rejecting a
/// confirmed fact is not the undo path (forget is), and a dismissed item is
/// already withdrawn. Matches the legacy `Candidate`-only legal set.
pub(crate) async fn reject(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<ContractClaim, ContractOpError> {
    let item = store
        .get_knowledge_item(id.as_str())
        .await?
        .ok_or(ContractOpError::NotFound)?;
    if item.state != KnowledgeState::Pending {
        return Err(ContractOpError::Conflict);
    }
    store
        .update_knowledge_item_state(id.as_str(), KnowledgeState::Dismissed)
        .await?;
    let item = store
        .get_knowledge_item(id.as_str())
        .await?
        .ok_or(ContractOpError::NotFound)?;
    ContractClaim::from_knowledge_item(&item).ok_or(ContractOpError::Unavailable)
}

/// Forgets `id`: withdrawn to `Dismissed` and its content erased in the same
/// transaction, keeping the row as a tombstone. The deletion promise
/// (`docs/memory.md` §Deletion) is that a forgotten fact's payload and
/// referenced columns are cleared while the row itself remains — so "why did
/// SAYA stop using that?" stays answerable and a re-remember of the same fact
/// reports *previously forgotten* rather than silently resurrecting it.
///
/// Erasing the value is coupled to `remember`'s dedup: the tombstone's value is
/// blanked, so dedup cannot key on the decoded value (a multi-valued slot keys
/// on value). `remember` dedups by the row id the put would land on, which is
/// value-independent for a single-valued slot and which the tombstone keeps for
/// a multi-valued one, so a re-remember still hits the tombstone. Erasing
/// without that change would silently resurrect a forgotten multi-valued fact;
/// the two move together.
///
/// `reason` is accepted for the command layer's signature; the new store has no
/// per-forget event, so it is not echoed anywhere (fail-closed: never surface
/// untrusted input). An unknown id is `NotFound`, matching the legacy refusal.
pub(crate) async fn forget(
    store: &SqliteStateStore,
    id: &ClaimId,
    _reason: saya_store::ForgetReason,
) -> Result<(), ContractOpError> {
    store.forget_knowledge_item(id.as_str()).await?;
    Ok(())
}

/// The schema known for `profile_id` as a three-state [`SchemaAvailability`]:
/// the cached tree, `Missing` (no cache entry), or `Unavailable` (store error).
/// The same construction `commands::cached_schema_availability` uses, kept here
/// so the operations layer can resolve an item's schema without reaching up
/// into the presentation layer (`commands` depends on `contracts`, not the
/// reverse).
async fn schema_availability_for(store: &SqliteStateStore, profile_id: &str) -> SchemaAvailability {
    match store.get_schema(profile_id).await {
        Ok(Some(cached)) => SchemaAvailability::available(cached.schema, cached.updated_unix_ms),
        Ok(None) => SchemaAvailability::Missing,
        Err(_) => SchemaAvailability::Unavailable,
    }
}

/// The cached schema to revalidate against, but only when it carries real
/// schema information. `Missing`, `Unavailable`, and an `Available` tree with
/// no `databases` (the no-op sentinel a fresh store writes) all return `None`:
/// an empty cache is absence of evidence, not evidence the object is gone, so
/// it routes to the no-schema arm of [`confirm`] rather than reading
/// `ObjectGone`. Mirrors `contracts_remember_schema::resolved_against`, which
/// treats an empty `databases` list as `NoSchema` for the same reason.
fn usable_table_schema(availability: &SchemaAvailability) -> Option<&saya_types::SchemaTree> {
    match availability.live_table_schema(SchemaFreshness::Unbounded) {
        Some(schema) if !schema.databases.is_empty() => Some(schema),
        _ => None,
    }
}

/// The live table for `object` in `schema`, if present.
fn find_table<'s>(
    object: &DatabaseObjectRef,
    schema: &'s saya_types::SchemaTree,
) -> Option<&'s Table> {
    schema.find_table(object.catalog(), object.schema(), object.object())
}

/// The structural dependency to validate `item` against. The stored
/// `schema_binding_json` is the record the item was written with; re-deriving
/// from `(slot, value)` refreshes it to the current binding format (the payload
/// and slot have not changed) so a confirm also repairs an item whose stored
/// binding this build could no longer deserialize. If the slot and payload
/// disagree — only possible for a corrupt row the store's own gate refuses —
/// fail closed rather than confirm an item we cannot interpret.
fn binding_to_validate(item: &KnowledgeItem) -> Result<SchemaBinding, ContractOpError> {
    if let Ok(binding) = serde_json::from_str::<SchemaBinding>(&item.schema_binding_json) {
        return Ok(binding);
    }
    SchemaBinding::derive(&item.slot, &item.value).ok_or(ContractOpError::Invalid)
}
