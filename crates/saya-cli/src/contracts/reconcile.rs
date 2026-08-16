//! Reconciliation writes: persist the 5b verdict. See the SPEC REVIEW in the
//! task report for the two gaps (truncation has no field in the spec's struct;
//! a stale claim must surface in the review queue, which 3d left candidates-only).
//!
//! Only `Stale` is written — `NeedsReview` is a *computed opinion* about a
//! moment that a later refresh may reverse, so persisting it would freeze a
//! maybe into a status a human must clear by hand (spec §1). A profile whose
//! live schema could not be read is SKIPPED, never marked: marking claims stale
//! over a network blip would destroy a user's accumulated knowledge (spec §2).

use crate::contracts::ContractOpError;
use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::validity::schema_state_for;
use crate::contracts::view::ContractSchemaState;
use saya_store::{ContractStore, SqliteStateStore};
use saya_types::{ClaimId, ClaimStatus, ProfileIdentity, SchemaTree};

/// The most claims one pass will examine. A bound keeps an explicit refresh's
/// cost predictable; the pass truncates and reports when more examinable claims
/// remain. It caps *examination* — claims skipped for a missing live schema are
/// counted but never consume it.
const RECONCILE_MAX_CLAIMS: usize = 1000;

#[derive(Debug)]
pub(crate) struct ReconcileOutcome {
    pub examined: usize,
    pub marked_stale: usize,
    pub skipped_unavailable: usize,
    /// `true` when the bound dropped at least one examinable claim. The spec's
    /// struct omits this; test 4.8 ("truncation is reported") requires it, so it
    /// is added here — the only deviation from the spec's `ReconcileOutcome`.
    pub truncated: bool,
}

impl ReconcileOutcome {
    /// The pass had no effect worth surfacing: no claim was examined, marked,
    /// or skipped, and the bound did not truncate. The refresh wiring uses this
    /// to stay silent on a clean refresh — a "0 / 0 / 0" line on every refresh
    /// is noise, and a clean refresh has long been a stderr-clean event.
    pub(crate) fn is_trivial(&self) -> bool {
        self.examined == 0
            && self.marked_stale == 0
            && self.skipped_unavailable == 0
            && !self.truncated
    }
}

/// Runs the 5b rule over every `Candidate`/`Confirmed` claim of `profiles`
/// against the supplied live `schemas`, and persists `Stale` (and only `Stale`)
/// in a single batched transaction. A profile absent from `schemas` is skipped
/// — its claims are counted in `skipped_unavailable` and never marked.
///
/// Idempotent: a claim already `Stale` is not `Candidate`/`Confirmed`, so a
/// second pass does not re-examine it (and the batch's legal check skips an
/// already-stale claim rather than hitting a `Conflict`). One claim failing to
/// transition does not abort the run — it is counted as examined and not marked.
///
/// Examination uses one [`ContractStore::list_claims_for_profile`] per profile
/// (not one `list_claims` per object), and the claims that compute `Stale` are
/// marked in one [`ContractStore::mark_stale_batch`] transaction — every
/// transition and its audit event commit together, so a store error rolls the
/// whole batch back and leaves no partial state where the per-claim path left
/// earlier claims already committed.
pub(crate) async fn reconcile(
    store: &SqliteStateStore,
    profiles: &[ProfileIdentity],
    schemas: &[(ProfileIdentity, SchemaTree)],
) -> Result<ReconcileOutcome, ContractOpError> {
    let mut outcome = ReconcileOutcome {
        examined: 0,
        marked_stale: 0,
        skipped_unavailable: 0,
        truncated: false,
    };
    let mut to_mark: Vec<ClaimId> = Vec::new();
    for profile in profiles {
        let live = live_schema(schemas, profile);
        // Reconcile is handed a live schema just fetched from the connector,
        // so it is current by construction — `Unbounded` freshness, never
        // age-gated. Built once per profile (not per claim) so a large tree is
        // not cloned for every claim in the pass.
        let availability = live.map(|schema| SchemaAvailability::available(schema.clone(), 0));
        for claim in store.list_claims_for_profile(profile).await? {
            // Only Candidate/Confirmed are re-examined. Rejected, Forgotten
            // and already-Stale claims are left alone — the last is why a
            // second pass is a no-op rather than a `Conflict` storm.
            if !matches!(
                claim.status,
                ClaimStatus::Candidate | ClaimStatus::Confirmed
            ) {
                continue;
            }
            let Some(availability) = availability.as_ref() else {
                // No live schema for this profile: we could not look, so we
                // must not mark. The claim is counted as skipped, not
                // examined, and never touches the bound.
                outcome.skipped_unavailable += 1;
                continue;
            };
            if outcome.examined >= RECONCILE_MAX_CLAIMS {
                // An examinable claim we cannot look at without exceeding the
                // bound. The pass was truncated of real work — report it and
                // stop collecting, leaving this and later claims for a future
                // pass. The claims already collected still get marked below.
                outcome.truncated = true;
                break;
            }
            outcome.examined += 1;
            if schema_state_for(&claim, availability, SchemaFreshness::Unbounded)
                == ContractSchemaState::Stale
            {
                to_mark.push(claim.id);
            }
        }
        if outcome.truncated {
            break;
        }
    }
    // One transaction for every Stale verdict the pass collected. A claim
    // raced out of the legal set between snapshot and mark is skipped by the
    // batch (counted as examined, not marked), exactly as the per-claim
    // `mark_stale` would have refused with `Conflict`. A store error rolls the
    // whole batch back and surfaces — no partial state.
    outcome.marked_stale = store.mark_stale_batch(&to_mark).await?;
    Ok(outcome)
}

fn live_schema<'s>(
    schemas: &'s [(ProfileIdentity, SchemaTree)],
    profile: &ProfileIdentity,
) -> Option<&'s SchemaTree> {
    schemas
        .iter()
        .find(|(p, _)| p == profile)
        .map(|(_, tree)| tree)
}
