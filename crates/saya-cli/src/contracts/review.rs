//! Review operations: thin typed wrappers over [`ContractStore`]. No policy
//! beyond what the store already enforces. A confirm-on-propose shortcut is
//! deliberately absent (ADR 0002 §4): this layer passes the caller's
//! `initial_status` through and lets the store refuse anything but
//! `UserExplicit` storing confirmed.

use thiserror::Error;

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::conflict::conflicts_for;
use crate::contracts::retrieval::RetrievalPolicy;
use crate::contracts::validity::schema_state_for;
use crate::contracts::view::{ContractSchemaState, RetrievedContract};
use saya_store::{
    ContractStore, ForgetReason, ProposeClaim, ProposeOutcome, SqliteStateStore, StoreError,
    StoredClaim,
};
use saya_types::{ClaimId, ClaimPayload, DatabaseObjectRef};

/// Adapter-facing review errors. Payload-free, mapping [`StoreError`] so a store
/// variant added later does not silently become an unhandled case in an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub(crate) enum ContractOpError {
    #[error("the requested claim does not exist")]
    NotFound,
    #[error("the operation conflicts with an existing claim")]
    Conflict,
    #[error("the value is not valid for storage")]
    Invalid,
    #[error("the value exceeds a store limit")]
    Limit,
    #[error("the state store is unavailable")]
    Unavailable,
}

impl From<StoreError> for ContractOpError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound => Self::NotFound,
            StoreError::Conflict => Self::Conflict,
            StoreError::Invalid => Self::Invalid,
            StoreError::LimitExceeded => Self::Limit,
            StoreError::Unavailable | StoreError::VersionUnsupported => Self::Unavailable,
            // StoreError is #[non_exhaustive]; a future variant is a store
            // problem the adapter cannot route around, so it degrades to
            // Unavailable rather than becoming an unhandled case.
            _ => Self::Unavailable,
        }
    }
}

pub(crate) async fn propose(
    store: &SqliteStateStore,
    request: ProposeClaim,
) -> Result<ProposeOutcome, ContractOpError> {
    Ok(store.propose_claim(request).await?)
}

pub(crate) async fn confirm(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<StoredClaim, ContractOpError> {
    Ok(store.confirm_claim(id).await?)
}

pub(crate) async fn edit(
    store: &SqliteStateStore,
    id: &ClaimId,
    payload: ClaimPayload,
) -> Result<StoredClaim, ContractOpError> {
    Ok(store.edit_claim(id, payload).await?)
}

pub(crate) async fn reject(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<StoredClaim, ContractOpError> {
    Ok(store.reject_claim(id).await?)
}

pub(crate) async fn forget(
    store: &SqliteStateStore,
    id: &ClaimId,
    reason: ForgetReason,
) -> Result<(), ContractOpError> {
    store.forget_claim(id, reason).await?;
    Ok(())
}

/// Assembles one object's contract for display: its recallable claims, their
/// conflicts, and a schema state. Returns `None` when no recallable claim
/// remains.
///
/// `schema` is the schema known for the object's profile — a
/// [`SchemaAvailability`], so a missing or unreadable cache classifies
/// `LiveSchemaUnavailable` rather than collapsing to an empty tree that would
/// read `Stale`. The state is the worst verdict across the kept claims — the
/// same aggregation `recall` uses (`ContractSchemaState::aggregate`). A cached
/// tree is compared against each claim's stored fingerprint, so `show` reports
/// `current` / `needs_review` / `stale` like the agent recall path, not a
/// constant `LiveSchemaUnavailable`.
///
/// `policy` is the shared retrieval policy (see [`super::retrieval`]) and also
/// selects the freshness gate: [`RetrievalPolicy::ForModel`] — used by
/// `contract_read` — bounds cached-schema age against `now_unix_ms`, so a
/// stale-by-age cache cannot vouch for currency; [`RetrievalPolicy::ForHumanReview`]
/// — used by `contracts show` — is unbounded, so a reviewer sees what the cache
/// knows regardless of age. `ForModel` also does not hand the model a stale
/// contract's claim *payloads*: when the aggregated state is `Stale` it returns
/// a contract that still names the object and reports `schema_state: Stale` but
/// carries **no claims**, so the model learns the object is stale without
/// reading a gone-column claim as a current fact.
pub(crate) async fn show(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    schema: &SchemaAvailability,
    policy: RetrievalPolicy,
    now_unix_ms: i64,
) -> Result<Option<RetrievedContract>, ContractOpError> {
    let claims = store.list_claims(object, &[]).await?;
    let recallable: Vec<StoredClaim> = claims
        .into_iter()
        .filter(|c| c.status.is_recallable())
        .collect();
    if recallable.is_empty() {
        return Ok(None);
    }
    let freshness = match policy {
        RetrievalPolicy::ForModel => SchemaFreshness::for_model(now_unix_ms),
        RetrievalPolicy::ForHumanReview => SchemaFreshness::Unbounded,
    };
    let schema_state = recallable
        .iter()
        .map(|c| schema_state_for(c, schema, freshness))
        .fold(ContractSchemaState::Current, |acc, s| acc.aggregate(s));
    let conflicts = conflicts_for(&recallable);
    // The same policy `recall` applies: a model-facing caller does not receive a
    // stale contract's claims. Unlike `recall` (which drops the contract and
    // counts it) `show` returns the object with `Stale` and an empty claim list,
    // so `contract_read` can tell the model *why* there is nothing to act on
    // rather than silently returning an empty result.
    let (claims, conflicts) =
        if policy == RetrievalPolicy::ForModel && schema_state == ContractSchemaState::Stale {
            (Vec::new(), Vec::new())
        } else {
            (recallable, conflicts)
        };
    Ok(Some(RetrievedContract {
        object: object.clone(),
        schema_state,
        claims,
        conflicts,
        truncated: false,
    }))
}
