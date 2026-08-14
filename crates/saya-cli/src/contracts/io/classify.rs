//! Per-claim classification for contract import (slice 6b).
//!
//! Each discovered claim is sorted into one of four verdicts before anything is
//! written, so `--dry-run` and a real import report from the same pre-scan:
//!
//! - `Stale` — the live (cached) schema lacks the object or a referenced column.
//!   Checked first and never stored: a file claiming a nonexistent column must
//!   not import silently (spec §1).
//! - `Duplicate` — an existing claim has the same dedup identity *and* the same
//!   value. Reported with the existing claim's real status (a duplicate of a
//!   forgotten claim reads forgotten, not success).
//! - `Conflicting` — an existing claim has the same dedup identity but a
//!   *different* value (e.g. the file says a column's role is `measure` while
//!   the store holds `identifier`). The store would refuse to store a second
//!   distinct value for that identity; we report the conflict and store
//!   nothing, leaving the existing claim untouched.
//! - `Added` — no existing claim shares the dedup identity; a new claim is
//!   stored.
//!
//! "Live schema" for the headless adapter is the store's *cached* schema for
//! the profile, the same source recall uses. A profile with no cached schema
//! cannot be checked, so no claim of that profile is marked stale here — a later
//! reconcile pass is where drift is caught, matching the reconcile module's
//! rule that a missing live schema skips marking rather than destroying
//! knowledge.

use saya_store::{ContractStore, StoredClaim};
use saya_types::{ClaimPayload, DatabaseObjectRef, SchemaTree};

/// One claim's import verdict. `Existing` claims carry the id and status of the
/// claim already in the store so the report can name them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImportVerdict {
    Added,
    Duplicate { existing_status: String },
    Conflicting { existing_id: String },
    Stale,
}

impl ImportVerdict {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Duplicate { .. } => "duplicate",
            Self::Conflicting { .. } => "conflicting",
            Self::Stale => "stale",
        }
    }
}

/// Classify one discovered `payload` for `object` against the claims already in
/// the store and the optional live (cached) `schema`. Pure: writes nothing.
pub(crate) async fn classify(
    store: &dyn ContractStore,
    object: &DatabaseObjectRef,
    payload: &ClaimPayload,
    schema: Option<&SchemaTree>,
) -> Result<ImportVerdict, saya_store::StoreError> {
    if is_stale(payload, object, schema) {
        return Ok(ImportVerdict::Stale);
    }
    let existing = store.list_claims(object, &[]).await?;
    let file_key = dedup_key(payload);
    for claim in &existing {
        let Some(existing_payload) = claim.payload.as_ref() else {
            continue;
        };
        if dedup_key(existing_payload) == file_key {
            return Ok(verdict_for_existing(claim, payload));
        }
    }
    Ok(ImportVerdict::Added)
}

/// Decide `Duplicate` vs `Conflicting` for an existing claim that shares the
/// dedup identity with the file's payload. Same value → duplicate (carrying the
/// real status so a duplicate of a forgotten claim reads forgotten); a
/// different value → conflicting.
fn verdict_for_existing(existing: &StoredClaim, file_payload: &ClaimPayload) -> ImportVerdict {
    match existing.payload.as_ref() {
        Some(existing_payload) if existing_payload == file_payload => ImportVerdict::Duplicate {
            existing_status: existing.status.as_str().into(),
        },
        _ => ImportVerdict::Conflicting {
            existing_id: existing.id.as_str().to_string(),
        },
    }
}

/// The dedup identity of a payload, matching the store's `deduplication_key`.
/// Two claims with the same key occupy the same slot; whether they agree is a
/// separate payload-equality check.
fn dedup_key(payload: &ClaimPayload) -> saya_store::DeduplicationKey {
    let serialized = serde_json::to_string(payload).unwrap_or_default();
    saya_store::deduplication_key(payload, &serialized)
}

/// Stale for import: a *real* live schema is present, and either the object is
/// absent or one of the claim's referenced columns is absent by name. No cached
/// schema ⇒ cannot determine ⇒ not stale (a later reconcile pass catches drift,
/// never a reason to drop knowledge here). An empty cached schema
/// (`databases` empty — the no-op sentinel `store_at` and a never-populated
/// cache both produce) carries no real schema information, so it is treated
/// the same as no schema: nothing is marked stale against a schema that says
/// nothing.
fn is_stale(
    payload: &ClaimPayload,
    object: &DatabaseObjectRef,
    schema: Option<&SchemaTree>,
) -> bool {
    let Some(schema) = schema else {
        return false;
    };
    if schema.databases.is_empty() {
        return false;
    }
    let Some(table) = live_table(schema, object) else {
        return true;
    };
    for name in payload.referenced_columns() {
        if !table
            .columns
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(name))
        {
            return true;
        }
    }
    false
}

fn live_table<'s>(
    schema: &'s SchemaTree,
    object: &DatabaseObjectRef,
) -> Option<&'s saya_types::Table> {
    schema
        .databases
        .iter()
        .find(|db| db.name.eq_ignore_ascii_case(object.catalog()))
        .and_then(|db| {
            db.schemas
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(object.schema()))
        })
        .and_then(|s| {
            s.tables
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(object.object()))
        })
}
