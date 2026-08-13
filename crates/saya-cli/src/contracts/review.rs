//! Review operations: thin typed wrappers over [`ContractStore`]. No policy
//! beyond what the store already enforces. A confirm-on-propose shortcut is
//! deliberately absent (ADR 0002 §4): this layer passes the caller's
//! `initial_status` through and lets the store refuse anything but
//! `UserExplicit` storing confirmed.

use thiserror::Error;

use crate::contracts::conflict::conflicts_for;
use crate::contracts::view::{ContractSchemaState, RetrievedContract};
use saya_store::{
    ContractStore, ForgetReason, ProposeClaim, ProposeOutcome, SqliteStateStore, StoreError,
    StoredClaim,
};
use saya_types::{ClaimId, ClaimPayload, DatabaseObjectRef, SchemaFingerprint};

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
/// remains. `show` has no live schema to compare against, so the state is
/// `LiveSchemaUnavailable`; a later slice with a live tree recomputes it.
pub(crate) async fn show(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    _fingerprint: &SchemaFingerprint,
) -> Result<Option<RetrievedContract>, ContractOpError> {
    let claims = store.list_claims(object, &[]).await?;
    let recallable: Vec<StoredClaim> = claims
        .into_iter()
        .filter(|c| c.status.is_recallable())
        .collect();
    if recallable.is_empty() {
        return Ok(None);
    }
    let conflicts = conflicts_for(&recallable);
    Ok(Some(RetrievedContract {
        object: object.clone(),
        schema_state: ContractSchemaState::LiveSchemaUnavailable,
        claims: recallable,
        conflicts,
        truncated: false,
    }))
}
