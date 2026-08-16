//! `use_candidate_once`: admit one candidate to a single recall without
//! confirming it. Split from [`super::review`] by concern — this is the one
//! review operation that writes nothing to the store, so it stands apart from
//! the confirm/reject/edit/forget wrappers that mutate a claim.

use super::op_error::ContractOpError;
use saya_store::{ContractStore, SqliteStateStore};
use saya_types::{ClaimId, ClaimStatus};

/// Admits one candidate claim to a single recall, without confirming it.
///
/// Reads the claim and refuses unless it is a live `Candidate` — rejected,
/// forgotten, stale, contradicted, and confirmed claims all return
/// [`ContractOpError::NotACandidate`]. The operation writes **nothing** to the
/// store: no status flip, no fingerprint change, no audit event, no evidence.
/// The claim keeps its `Candidate` status and `AssistantInferred` origin, so a
/// user who uses one and never returns finds it exactly as it was (spec C §3).
///
/// The admission itself is not a persisted thing; it is request-scoped. The
/// caller threads the validated id into the next [`RecallRequest`]'s
/// `admit_candidate`, which [`selection`](super::selection) honours for that
/// one recall only — the request is built and dropped per turn, so an admission
/// cannot outlive the turn it was made for (spec C §4 — one turn, in-memory).
/// Because the claim stays `Candidate`, the render layer still marks it
/// `[candidate — unconfirmed]` when supplied: being chosen for one turn confers
/// no authority (spec C §3, ADR 0002 §4).
///
/// Fail-soft: a store read failure returns a typed error and damages nothing —
/// nothing was written.
pub(crate) async fn use_candidate_once(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<(), ContractOpError> {
    let claim = store
        .get_claim(id)
        .await?
        .ok_or(ContractOpError::NotFound)?;
    if claim.status != ClaimStatus::Candidate {
        return Err(ContractOpError::NotACandidate);
    }
    Ok(())
}
