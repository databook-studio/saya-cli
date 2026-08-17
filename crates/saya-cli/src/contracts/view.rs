//! Typed results every adapter renders. Mirrors plan §6.4. No formatting lives
//! here — these structs carry data and IDs only.

use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, DatabaseObjectRef, KnowledgeState,
};

/// The render carrier a `RetrievedContract` carries per claim — the fields
/// render, the receipt, and conflict detection read. Both stores project to
/// it: the recall path from a `KnowledgeItem` (D-3), the review path from a
/// `StoredClaim` (legacy, until its later chunk). Keeping one carrier means
/// `render_body`, `supplied_contracts`, and `conflicts_for` do not branch on
/// which store a contract came from.
///
/// `id` is the row id wrapped in a [`ClaimId`] (a `ki-…` knowledge id parses,
/// as does a legacy `c-…` claim id) so the dispute marker match and the
/// receipt keep using [`ClaimId::as_str`]. `value` is the payload (never
/// `None` — a `StoredClaim` whose payload did not decode is dropped at
/// projection, not carried as a blank line). `status` is the rendered
/// vocabulary (`Confirmed`/`Candidate`) the in-band marker and the receipt
/// carry.
pub(crate) struct ContractClaim {
    pub id: ClaimId,
    #[allow(dead_code)]
    pub object: DatabaseObjectRef,
    pub value: ClaimPayload,
    pub source: ClaimOrigin,
    pub status: ClaimStatus,
}

impl ContractClaim {
    /// Projects a loaded [`saya_store::KnowledgeItem`] into the render carrier.
    /// The store row's string id is wrapped in a [`ClaimId`] (its `ki-…` form
    /// parses), and the persisted [`KnowledgeState`] is mapped to the
    /// [`ClaimStatus`] the render marker and receipt use. Returns `None` for
    /// an id that is not a valid `ClaimId` (a row written by an incompatible
    /// build) so the caller drops it rather than surfacing a half-built line.
    pub(crate) fn from_knowledge_item(item: &saya_store::KnowledgeItem) -> Option<Self> {
        let id = ClaimId::parse(&item.id).ok()?;
        Some(Self {
            id,
            object: item.object.clone(),
            value: item.value.clone(),
            source: item.source,
            status: status_from_state(item.state),
        })
    }

    /// Projects a legacy [`StoredClaim`] into the render carrier for the
    /// review path (`show`), which still reads `contract_claims` until its
    /// later chunk. A claim whose payload did not decode (`None`) is dropped:
    /// the render layer never drew a line for one anyway.
    pub(crate) fn from_stored_claim(claim: &saya_store::StoredClaim) -> Option<Self> {
        let value = claim.payload.clone()?;
        Some(Self {
            id: claim.id.clone(),
            object: claim.object.clone(),
            value,
            source: claim.origin,
            status: claim.status,
        })
    }
}

/// The [`ClaimStatus`] a persisted [`KnowledgeState`] renders as on the recall
/// path. `Active → Confirmed` (a binding fact), `Pending → Candidate` (an
/// unconfirmed inference), `Dismissed → Rejected` (withdrawn; never reaches
/// render — admissibility excludes it, and validity reads it `Invalid`).
/// Kept here beside the carrier so render and the receipt share one mapping.
pub(crate) fn status_from_state(state: KnowledgeState) -> ClaimStatus {
    match state {
        KnowledgeState::Active => ClaimStatus::Confirmed,
        KnowledgeState::Pending => ClaimStatus::Candidate,
        KnowledgeState::Dismissed => ClaimStatus::Rejected,
        // `KnowledgeState` is `#[non_exhaustive]`; a future variant the render
        // layer does not know about fails closed to a non-recallable status
        // rather than guessing an authority it does not have.
        _ => ClaimStatus::Rejected,
    }
}

/// One object's recallable contract, ready to render.
pub(crate) struct RetrievedContract {
    pub object: DatabaseObjectRef,
    /// Worst state across the contract's claims (see [`ContractSchemaState::aggregate`]).
    pub schema_state: ContractSchemaState,
    pub claims: Vec<ContractClaim>,
    pub conflicts: Vec<ContractConflict>,
    pub truncated: bool,
}

/// Validity of a contract's claims against the live schema.
///
/// Ordered worst-first for aggregation via [`Self::aggregate`]:
/// `Stale` beats `LiveSchemaUnavailable` beats `NeedsReview` beats `Current`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContractSchemaState {
    Current,
    NeedsReview,
    LiveSchemaUnavailable,
    Stale,
}

impl ContractSchemaState {
    /// A claim cannot be more reassuring than the worst claim in the same
    /// contract — one stale claim must flag the object, not hide behind a
    /// `Current` sibling.
    pub(crate) fn aggregate(self, other: Self) -> Self {
        let rank = |state: Self| match state {
            Self::Stale => 3,
            Self::LiveSchemaUnavailable => 2,
            Self::NeedsReview => 1,
            Self::Current => 0,
        };
        if rank(self) >= rank(other) {
            self
        } else {
            other
        }
    }
}

/// A disagreement between confirmed claims of an exclusive kind on one object.
/// Names the kind and the claim IDs only — never claim text.
pub(crate) struct ContractConflict {
    pub kind: &'static str,
    pub claim_ids: Vec<ClaimId>,
}

/// The full result of a recall: the selected contracts plus diagnostics that
/// answer plan §16's "why these and not others" without carrying any claim text.
pub(crate) struct RecallOutcome {
    pub contracts: Vec<RetrievedContract>,
    pub diagnostics: RecallDiagnostics,
}

/// Counts only — never claim text, IDs, or payloads. A test asserts this.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RecallDiagnostics {
    pub attempted: bool,
    pub considered: usize,
    pub selected: usize,
    pub excluded_by_privacy: usize,
    pub excluded_by_status: usize,
    /// Claims a schema-staleness *policy* dropped (a contract that aggregates
    /// to `Stale` on the model path). A policy decision, not a bound — kept
    /// distinct from [`Self::excluded_by_count_bounds`] so the two cannot be
    /// read as each other. Surfaced to the model by `contract_search` to pick
    /// "matched but stale" apart from "matched nothing".
    pub excluded_by_schema: usize,
    /// Claims dropped by a count bound — objects beyond `max_objects` (all
    /// their claims) and claims beyond `max_claims_per_object` within a kept
    /// contract. A bound, not a policy decision, so it lives apart from
    /// [`Self::excluded_by_schema`]. The P1a recall receipt reads this to make
    /// truncation visible (`dropped_by_bounds`).
    pub excluded_by_count_bounds: usize,
    pub store_unavailable: bool,
}
