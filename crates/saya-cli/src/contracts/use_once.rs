//! `use_candidate_once`: admit one candidate to a single recall without
//! confirming it. Split from [`super::review`] by concern — this is the one
//! review operation that writes nothing to the store, so it stands apart from
//! the confirm/reject/forget wrappers that mutate an item.

use super::op_error::ContractOpError;
use saya_store::{KnowledgeItemStore, SqliteStateStore};
use saya_types::{ClaimId, KnowledgeState};

/// Admits one candidate item to a single recall, without confirming it.
///
/// Reads the item and refuses unless it is a live `Pending` candidate — an
/// `Active` (already admissible by the mode) or `Dismissed` item returns
/// [`ContractOpError::NotACandidate`]. The operation writes **nothing** to the
/// store: no state flip, no binding change, no fingerprint change, no audit.
/// The item keeps its `Pending` state and `AssistantInferred` origin, so a
/// user who uses one and never returns finds it exactly as it was.
///
/// The admission itself is not a persisted thing; it is request-scoped. The
/// caller threads the validated id into the next [`RecallRequest`]'s
/// `admit_candidate`, which [`selection`](super::selection) honours for that
/// one recall only — the request is built and dropped per turn, so an admission
/// cannot outlive the turn it was made for.
/// Because the item stays `Pending`, the render layer still marks it
/// `[candidate — unconfirmed]` when supplied: being chosen for one turn confers
/// no authority.
///
/// Fail-soft: a store read failure returns a typed error and damages nothing —
/// nothing was written.
pub(crate) async fn use_candidate_once(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<(), ContractOpError> {
    let item = store
        .get_knowledge_item(id.as_str())
        .await?
        .ok_or(ContractOpError::NotFound)?;
    if item.state != KnowledgeState::Pending {
        return Err(ContractOpError::NotACandidate);
    }
    Ok(())
}
