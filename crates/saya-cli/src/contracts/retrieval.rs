//! The retrieval policy: who the result is for, and what that means for a
//! contract whose computed schema state is `Stale`.
//!
//! This is the single place that decides whether a computed-stale contract
//! reaches the caller. Three model-facing call sites — prompt recall,
//! `contract_search`, `contract_read` — all pass [`RetrievalPolicy::ForModel`]
//! and so drop a contract whose claims aggregate to `Stale`. The human-review
//! sites — `contracts list`/`show`/`queue`, import reporting — pass
//! [`RetrievalPolicy::ForHumanReview`] and keep stale contracts, their
//! fingerprints and the reason they are stale, because that *is* the point of a
//! review path.
//!
//! Only *computed* `Stale` is excluded here. A claim whose persisted status is
//! already `Stale` never reaches this layer — `is_recallable` excluded it in
//! selection. `NeedsReview` is **not** excluded: if it were, one unrelated
//! column added to a wide table would silently mute every claim on that table,
//! and `Stale` and `NeedsReview` would stop meaning different things.
//! `LiveSchemaUnavailable` is not excluded either — "could not read the schema"
//! is not "a column is gone", and hiding it would let an unreadable cache pass
//! as an empty memory.

use crate::contracts::view::{ContractSchemaState, RecallDiagnostics, RetrievedContract};

/// Who a retrieval's result is for. See the module docs for the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RetrievalPolicy {
    /// A result shown to the model. A contract that aggregates to `Stale` is
    /// dropped entirely and counted in `excluded_by_schema` — the exclusion is
    /// not silent, so a user can see why a fact they remembered stopped
    /// appearing. Every other state (including `NeedsReview`) is kept.
    #[default]
    ForModel,
    /// A result a human is reviewing. Nothing is dropped here; a stale
    /// contract is kept with its state and reason so the reviewer can act on
    /// it. This is the policy for `contracts list`/`show`/`queue`.
    ForHumanReview,
}

/// Drops computed-`Stale` contracts from `contracts` when `policy` is
/// [`RetrievalPolicy::ForModel`], counting each dropped contract in
/// `diag.excluded_by_schema` so the exclusion is observable. `ForHumanReview`
/// returns `contracts` untouched.
///
/// Called once per retrieval path (`recall`, `show`) so every model-facing call
/// site gets the same rule without re-implementing it — the shared-operation
/// rule this bug violated.
pub(crate) fn apply(
    contracts: Vec<RetrievedContract>,
    policy: RetrievalPolicy,
    diag: &mut RecallDiagnostics,
) -> Vec<RetrievedContract> {
    if policy == RetrievalPolicy::ForHumanReview {
        return contracts;
    }
    let mut kept = Vec::with_capacity(contracts.len());
    for contract in contracts {
        if contract.schema_state == ContractSchemaState::Stale {
            // A computed-stale contract is not safe for the model to read as a
            // fact: a column it depends on is gone. Count it so a user can see
            // why a remembered fact disappeared, then drop it.
            diag.excluded_by_schema += 1;
            continue;
        }
        kept.push(contract);
    }
    kept
}
