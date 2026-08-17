//! The review queue: the opposite view of recall. Recall answers "what is
//! true about this question"; the queue answers "what is waiting for me". It
//! lists `Pending` items — the candidates a user has not yet decided on — with
//! the per-item schema state a human needs to decide.
//!
//! Ordering is `created_unix_ms` ascending (oldest first), then the slot's
//! canonical string, then the item id — a queue whose order shifts between runs
//! is one a user cannot work through. The legacy queue sorted by
//! `evidence_count` from `contract_evidence`; that table is gone by design (a
//! successful query is not evidence a business definition is true), so the
//! queue no longer orders by evidence and the carried `evidence_count` is
//! always zero — kept on the carrier only until the presentation layer (a
//! later chunk) drops the field from its render DTO.
//!
//! The queue reads `knowledge_items` via [`KnowledgeItemStore`] — the same
//! rows the harness-owned learning path writes — so a candidate the model
//! inferred this turn is in the queue the next. One store round trip per
//! profile ([`KnowledgeItemStore::knowledge_for_profile`]); the `Pending`
//! filter and the schema-state computation stay in Rust, per-item, because a
//! reviewer judging a single candidate needs that candidate's own validity,
//! not a worst-case merge with its siblings.

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::knowledge_validity::item_validity_for;
use crate::contracts::view::ContractSchemaState;
use saya_store::{KnowledgeItem, KnowledgeItemStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimPayload, ClaimStatus, DatabaseObjectRef, KnowledgeState, ProfileIdentity,
};

use crate::contracts::ContractOpError;

/// The largest queue a single read returns. A reviewer works the top of the
/// list; beyond this the list stops being a queue and becomes an archive.
pub(crate) const QUEUE_LIMIT_CAP: usize = 200;

/// The default queue size when the adapter does not name one. Capped well
/// under [`QUEUE_LIMIT_CAP`] so an unbounded `saya contracts queue` still
/// returns a workable list, not the whole archive.
pub(crate) const QUEUE_DEFAULT_LIMIT: usize = 50;

/// The render carrier for one queued item. Carries the fields the
/// presentation layer's `queue_item_view` reads (`payload`, `id`, `status`,
/// `object`) projected from the [`KnowledgeItem`] — plus the `schema_state`
/// and `evidence_count` that ride on [`QueuedCandidate`]. A dedicated carrier
/// rather than the recall path's `ContractClaim` because the queue view reads
/// `payload: Option<ClaimPayload>` (a forgotten-tombstone fallback the recall
/// carrier's non-optional `value` does not model); a dedicated carrier rather
/// than the legacy `StoredClaim` because reconstructing one would fabricate a
/// fingerprint and referenced columns the queue never reads. Built from a
/// `KnowledgeItem`, so it leaks nothing the store does not.
#[derive(Debug)]
pub(crate) struct QueuedClaim {
    pub payload: Option<ClaimPayload>,
    pub id: ClaimId,
    pub status: ClaimStatus,
    pub object: DatabaseObjectRef,
}

impl QueuedClaim {
    /// Projects a loaded [`KnowledgeItem`] into the queue carrier. An item id
    /// that is not a valid `ClaimId` (a row written by an incompatible build)
    /// returns `None` so the caller drops it rather than surfacing a half-built
    /// line — the same drop `ContractClaim::from_knowledge_item` applies.
    pub(crate) fn from_item(item: &KnowledgeItem) -> Option<Self> {
        let id = ClaimId::parse(&item.id).ok()?;
        Some(Self {
            payload: Some(item.value.clone()),
            id,
            status: status_for_queue(item.state),
            object: item.object.clone(),
        })
    }
}

/// The [`ClaimStatus`] a queued `Pending` item renders as. `Pending → Candidate`
/// is the only state that reaches the queue (the filter excludes the rest), so
/// the other arms are defensive — a `#[non_exhaustive]` future state fails
/// closed to a non-candidate status rather than guessing an authority it does
/// not have. Mirrors [`crate::contracts::view::status_from_state`].
fn status_for_queue(state: KnowledgeState) -> ClaimStatus {
    match state {
        KnowledgeState::Active => ClaimStatus::Confirmed,
        KnowledgeState::Pending => ClaimStatus::Candidate,
        KnowledgeState::Dismissed => ClaimStatus::Rejected,
        _ => ClaimStatus::Rejected,
    }
}

/// One item waiting for review — a `Pending` candidate. Carries the projected
/// claim, the per-item schema state (a candidate about a table that has since
/// changed says so), and the evidence count. The evidence *rows* are gone
/// (`contract_evidence` is dropped by design), so `evidence_count` is always
/// zero; it stays on the carrier until the presentation layer drops it.
#[derive(Debug)]
pub(crate) struct QueuedCandidate {
    pub claim: QueuedClaim,
    pub schema_state: ContractSchemaState,
    pub evidence_count: usize,
}

/// The candidate review queue for `profiles`. `Pending` items only, ordered
/// oldest first, then by slot, then by id string — a queue whose order shifts
/// between runs is one a user cannot work through. `limit` is clamped to
/// [`QUEUE_LIMIT_CAP`] on the upper bound; `0` is an empty queue, not clamped
/// up, because "nothing waiting" is a legitimate answer.
///
/// One store round trip per profile: a single
/// [`KnowledgeItemStore::knowledge_for_profile`] fetches every item. The
/// `Pending` filter and the per-item schema state stay in Rust — the schema
/// state is per-candidate (not the per-object aggregate recall uses), because
/// a reviewer judging one candidate needs that candidate's own validity.
pub(crate) async fn review_queue(
    store: &SqliteStateStore,
    profiles: &[ProfileIdentity],
    schemas: &[(ProfileIdentity, SchemaAvailability)],
    limit: usize,
) -> Result<Vec<QueuedCandidate>, ContractOpError> {
    let limit = limit.min(QUEUE_LIMIT_CAP);
    // Collect the Pending items with their per-item schema state first, so the
    // sort can key on the item's created stamp and slot (fields the render
    // carrier does not carry) before projecting to the carrier.
    let mut entries: Vec<(KnowledgeItem, ContractSchemaState)> = Vec::new();
    for profile in profiles {
        for item in store.knowledge_for_profile(profile).await? {
            if item.state != KnowledgeState::Pending {
                continue;
            }
            let live = live_schema(schemas, item.object.profile());
            // The queue is a human-review path: Unbounded freshness, so a
            // stale-by-age cache still classifies what it knows. A reviewer
            // is not being asked to trust a query built on these contracts.
            let schema_state = item_validity_for(&item, live, SchemaFreshness::Unbounded).into();
            entries.push((item, schema_state));
        }
    }
    // Oldest first (created_unix_ms asc), then slot's canonical string, then
    // id — deterministic across runs. `KnowledgeSlot` has no `Ord` derive, so
    // the slot keys by its `as_str` canonical identifier.
    entries.sort_by(|(a, _), (b, _)| {
        a.created_unix_ms
            .cmp(&b.created_unix_ms)
            .then_with(|| a.slot.as_str().cmp(&b.slot.as_str()))
            .then_with(|| a.id.cmp(&b.id))
    });
    entries.truncate(limit);
    let queued = entries
        .into_iter()
        .filter_map(|(item, schema_state)| {
            // An item id that is not a valid `ClaimId` (a row from an
            // incompatible build) is dropped here, not rendered as a blank line.
            let claim = QueuedClaim::from_item(&item)?;
            Some(QueuedCandidate {
                claim,
                schema_state,
                // contract_evidence is gone; there is no evidence to count.
                evidence_count: 0,
            })
        })
        .collect();
    Ok(queued)
}

fn live_schema<'s>(
    schemas: &'s [(ProfileIdentity, SchemaAvailability)],
    profile: &ProfileIdentity,
) -> &'s SchemaAvailability {
    schemas
        .iter()
        .find(|(p, _)| p == profile)
        .map(|(_, avail)| avail)
        .unwrap_or(&SchemaAvailability::Missing)
}
