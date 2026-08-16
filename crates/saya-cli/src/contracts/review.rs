//! Review operations: thin typed wrappers over [`ContractStore`]. No policy
//! beyond what the store already enforces. A confirm-on-propose shortcut is
//! deliberately absent (ADR 0002 §4): this layer passes the caller's
//! `initial_status` through and lets the store refuse anything but
//! `UserExplicit` storing confirmed.

use super::op_error::ContractOpError;

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::conflict::conflicts_for;
use crate::contracts::retrieval::RetrievalPolicy;
use crate::contracts::validity::schema_state_for;
use crate::contracts::view::{ContractSchemaState, RetrievedContract};
use saya_store::{
    ContractStore, ForgetReason, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore,
    StoredClaim,
};
use saya_types::{ClaimId, ClaimPayload, ClaimStatus, DatabaseObjectRef, Table};

/// Adapter-facing review errors. Payload-free, mapping [`StoreError`] so a store
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
    // A Stale claim cannot be confirmed by a status-only flip: the stored
    // fingerprint would stay untouched, so the next read would recompute the
    // digest, find it still differs, and return Stale again — a silent no-op.
    // Revalidate it against the live schema: rewrite the fingerprint and
    // snapshots in the same transaction as the status flip. The schema comes
    // from the claim's own profile (a confirm carries no --profile flag), read
    // from the same cache `remember`/`show` use. With no schema there is
    // nothing to revalidate against — refuse, do not guess.
    let claim = store
        .get_claim(id)
        .await?
        .ok_or(ContractOpError::NotFound)?;
    if claim.status != ClaimStatus::Stale {
        return Ok(store.confirm_claim(id).await?);
    }
    let availability = schema_availability_for(store, claim.object.profile().as_str()).await;
    let live_table = live_table_for(&claim, &availability)?.ok_or(ContractOpError::ObjectGone)?;
    // Name the actual obstacle before the store refuses generically. The store
    // still checks independently; this exists so the message tells a reviewer
    // which of the two repairs — edit or forget — applies to them.
    if first_missing_column(&claim, live_table).is_some() {
        return Err(ContractOpError::ColumnGone);
    }
    Ok(store.revalidate_claim(id, live_table).await?)
}

/// The schema known for `profile_id` as a three-state [`SchemaAvailability`]:
/// the cached tree, `Missing` (no cache entry), or `Unavailable` (store error).
/// The same construction `commands::cached_schema_availability` uses, kept here
/// so the operations layer can resolve a claim's schema without reaching up
/// into the presentation layer (`commands` depends on `contracts`, not the
/// reverse).
async fn schema_availability_for(store: &SqliteStateStore, profile_id: &str) -> SchemaAvailability {
    match store.get_schema(profile_id).await {
        Ok(Some(cached)) => SchemaAvailability::available(cached.schema, cached.updated_unix_ms),
        Ok(None) => SchemaAvailability::Missing,
        Err(_) => SchemaAvailability::Unavailable,
    }
}

/// The live table the claim's object resolves to in `availability`, or `None`
/// when no schema is available at all. A human is confirming, so the freshness
/// gate is unbounded — a reviewer is not asked to trust a query built on their
/// own contracts, and a confirm must work against whatever the cache knows.
fn live_table_for<'a>(
    claim: &StoredClaim,
    availability: &'a SchemaAvailability,
) -> Result<Option<&'a Table>, ContractOpError> {
    let Some(schema) = availability.live_table_schema(SchemaFreshness::Unbounded) else {
        // No schema to revalidate against. `Missing` and `Unavailable` both
        // land here; the caller refuses with `SchemaUnavailable` rather than
        // reviving a claim it cannot check.
        return Err(ContractOpError::SchemaUnavailable);
    };
    Ok(schema.find_table(
        claim.object.catalog(),
        claim.object.schema(),
        claim.object.object(),
    ))
}

/// The first referenced column absent from `live`, if any. Compared
/// case-insensitively, matching how validity classifies drift.
fn first_missing_column<'a>(claim: &'a StoredClaim, live: &Table) -> Option<&'a str> {
    claim.referenced_columns.iter().find_map(|col| {
        (!live
            .columns
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(&col.name)))
        .then_some(col.name.as_str())
    })
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
