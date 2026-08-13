//! Typed results every adapter renders. Mirrors plan §6.4. No formatting lives
//! here — these structs carry data and IDs only.

use saya_store::StoredClaim;
use saya_types::{ClaimId, DatabaseObjectRef};

/// One object's recallable contract, ready to render.
pub(crate) struct RetrievedContract {
    pub object: DatabaseObjectRef,
    /// Worst state across the contract's claims (see [`ContractSchemaState::aggregate`]).
    pub schema_state: ContractSchemaState,
    pub claims: Vec<StoredClaim>,
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
    pub excluded_by_schema: usize,
    pub store_unavailable: bool,
}
