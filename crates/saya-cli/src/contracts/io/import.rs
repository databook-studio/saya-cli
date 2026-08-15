//! Import orchestration: discovered claims → stored claims (slice 6b).
//!
//! [`import_contracts`] runs 6a's discovery, classifies each claim (see
//! [`super::classify`]), and — outside a dry run — proposes the `Added` ones
//! with `origin = TeamFile` and `initial_status = Confirmed`. ADR 0002 §4: a
//! reviewed team file enters confirmed within its declared scope, and the store
//! admits `TeamFile` as confirmable. Conflicts with local claims surface per
//! ADR decision 3 at recall, not as silent overwrites here.

use super::classify::ImportVerdict as V;
use super::{ImportClaimResult, ImportReport};
use crate::contracts::discover::{DiscoveredContract, discover_contracts};
use crate::contracts::review::ContractOpError;
use saya_store::{ContractStore, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore};
use saya_types::{ClaimOrigin, ClaimStatus, DatabaseObjectRef, ProfileIdentity, SchemaTree};

/// Import discovered contracts for `project_root` into `store`, bound to
/// `identity`. `dry_run` classifies and reports without writing. The cached
/// schema for `identity` is the live schema stale claims are checked against;
/// no cached schema ⇒ no claim is marked stale (drift is a later reconcile
/// pass, never a reason to drop knowledge here).
pub(crate) async fn import_contracts(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
    project_root: &std::path::Path,
    dry_run: bool,
) -> Result<ImportReport, ContractOpError> {
    let discovered = discover_contracts(project_root).map_err(|_| ContractOpError::Unavailable)?;
    let schema = cached_schema(store, identity).await?;
    let mut report = ImportReport {
        rejected: discovered.rejected,
        truncated_by: discovered.truncated_by,
        ..Default::default()
    };
    for contract in &discovered.contracts {
        let object = build_object(identity, contract).map_err(|_| ContractOpError::Invalid)?;
        for payload in &contract.claims {
            let verdict =
                super::classify::classify(store, &object, payload, schema.as_ref()).await?;
            let result = ImportClaimResult {
                source: contract.source.clone(),
                object: object.qualified_name(),
                verdict: verdict.clone(),
            };
            match verdict {
                V::Added => {
                    if !dry_run {
                        propose_imported(store, &object, payload, schema.as_ref()).await?;
                    }
                    report.added.push(result);
                }
                V::Duplicate { .. } => report.duplicates.push(result),
                V::Conflicting { .. } => report.conflicts.push(result),
                V::Stale => report.stale.push(result),
            }
        }
    }
    Ok(report)
}

/// The cached schema for `identity`, the live schema the stale check compares
/// against. A read failure is an unavailable store, not an empty schema.
async fn cached_schema(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
) -> Result<Option<SchemaTree>, ContractOpError> {
    match store.get_schema(identity.as_str()).await {
        Ok(Some(cached)) => Ok(Some(cached.schema)),
        Ok(None) => Ok(None),
        Err(_) => Err(ContractOpError::Unavailable),
    }
}

fn build_object(
    identity: &ProfileIdentity,
    contract: &DiscoveredContract,
) -> Result<DatabaseObjectRef, super::ArgError> {
    DatabaseObjectRef::new(
        identity.clone(),
        &contract.object.catalog,
        &contract.object.schema,
        &contract.object.object,
        saya_types::DatabaseObjectKind::Table,
    )
    .map_err(|_| super::ArgError::MalformedObject)
}

/// Propose one discovered claim as a confirmed `TeamFile` claim. A `Duplicate`
/// outcome is not an error here — the pre-scan already classified this as
/// `Added`, and the store returning `Duplicate` would only mean a claim with
/// the same dedup key appeared between the scan and the propose, which a
/// single-threaded CLI cannot observe; either way the claim is in the store.
///
/// When the cached `schema` contains the object, store the real
/// `SchemaFingerprint::of_table` digest and typed `referenced_column_snapshots`
/// — the same `remember` does — so a valid imported claim reads `current`, not
/// `needs_review`. The pre-scan already verified the object and every
/// referenced column are present for an `Added` verdict, so a present schema
/// guarantees a found table; the lookup is defensive anyway. No schema, an
/// empty cached tree, or an object the cache lacks keeps the unobserved
/// sentinel and name-only snapshots: there is nothing real to record against,
/// and a later reconcile catches drift.
async fn propose_imported(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: &saya_types::ClaimPayload,
    schema: Option<&SchemaTree>,
) -> Result<(), ContractOpError> {
    let table = schema
        .filter(|s| !s.databases.is_empty())
        .and_then(|s| s.find_table(object.catalog(), object.schema(), object.object()));
    let (fingerprint, referenced_columns) = match table {
        Some(table) => (
            saya_types::SchemaFingerprint::of_table(saya_types::DatabaseObjectKind::Table, table),
            payload.referenced_column_snapshots(table),
        ),
        None => (
            crate::commands::unobserved_fingerprint(),
            payload.referenced_column_name_snapshots(),
        ),
    };
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint,
        payload: payload.clone(),
        origin: ClaimOrigin::TeamFile,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns,
    };
    match store.propose_claim(request).await {
        Ok(ProposeOutcome::Stored(_)) | Ok(ProposeOutcome::Duplicate { .. }) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
