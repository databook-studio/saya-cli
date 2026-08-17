//! Assembling one object's contract for display, split from [`super::review`]
//! by concern: this module only *reads* (`knowledge_for_object`); the
//! confirm/reject/forget *mutations* live in `review.rs`. Chunk 3 moved this
//! onto `knowledge_items`, so `contracts show` and the `contract_read` agent
//! tool read the same table recall does — no split brain with the recall path.

use super::op_error::ContractOpError;
use super::view::{ContractClaim, ContractSchemaState, RetrievedContract};

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::conflict::conflicts_for;
use crate::contracts::knowledge_validity::item_validity_for;
use crate::contracts::retrieval::RetrievalPolicy;
use saya_store::{KnowledgeItem, KnowledgeItemStore, SqliteStateStore};
use saya_types::{DatabaseObjectRef, KnowledgeState};

/// Assembles one object's contract for display: its non-dismissed items, their
/// conflicts, and an aggregated schema state. Returns `None` when no
/// non-dismissed item remains.
///
/// `schema` is the schema known for the object's profile — a
/// [`SchemaAvailability`], so a missing or unreadable cache classifies
/// `LiveSchemaUnavailable` rather than collapsing to an empty tree that would
/// read `Stale`. The state is the worst verdict across the kept items — the
/// same aggregation `recall` uses. Validity is the D-4 binding model
/// ([`item_validity_for`]): an unrelated column changing never invalidates, a
/// gone bound column reads `Invalid` (→ `Stale`).
///
/// `policy` selects the freshness gate and the stale-claim rule:
/// [`RetrievalPolicy::ForModel`] — used by `contract_read` — bounds
/// cached-schema age against `now_unix_ms`, and a contract aggregating to
/// `Stale` is returned with an empty claim list so the model learns the object
/// is stale without reading a gone-column claim as a current fact;
/// [`RetrievalPolicy::ForHumanReview`] — used by `contracts show` — is
/// unbounded and keeps every claim so a reviewer can act on it.
pub(crate) async fn show(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    schema: &SchemaAvailability,
    policy: RetrievalPolicy,
    now_unix_ms: i64,
) -> Result<Option<RetrievedContract>, ContractOpError> {
    let items = store.knowledge_for_object(object).await?;
    let kept: Vec<&KnowledgeItem> = items
        .iter()
        .filter(|it| !matches!(it.state, KnowledgeState::Dismissed))
        .collect();
    if kept.is_empty() {
        return Ok(None);
    }
    let freshness = match policy {
        RetrievalPolicy::ForModel => SchemaFreshness::for_model(now_unix_ms),
        RetrievalPolicy::ForHumanReview => SchemaFreshness::Unbounded,
    };
    let mut state = ContractSchemaState::Current;
    let mut claims: Vec<ContractClaim> = Vec::with_capacity(kept.len());
    for item in &kept {
        let validity = item_validity_for(item, schema, freshness);
        state = state.aggregate(validity.into());
        if let Some(carrier) = ContractClaim::from_knowledge_item(item) {
            claims.push(carrier);
        }
    }
    let conflicts = conflicts_for(&claims);
    // The model-facing path does not hand the model a stale contract's claim
    // payloads: keep the object and its `Stale` state (so `contract_read` can
    // say *why* there is nothing to act on) but drop the claims. The
    // human-review path keeps everything — that is what a reviewer is here for.
    let (claims, conflicts) =
        if policy == RetrievalPolicy::ForModel && state == ContractSchemaState::Stale {
            (Vec::new(), Vec::new())
        } else {
            (claims, conflicts)
        };
    Ok(Some(RetrievedContract {
        object: object.clone(),
        schema_state: state,
        claims,
        conflicts,
        truncated: false,
    }))
}
