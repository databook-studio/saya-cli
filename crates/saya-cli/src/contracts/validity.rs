//! Read-time validity: a stored claim's fingerprint against the live schema.
//!
//! Computes only — reconciliation writes are Phase 5. See the SPEC REVIEW in the
//! report for the deviation on retyped / nullability-changed referenced columns:
//! the live `SchemaTree` carries no stored per-column type or nullability to
//! compare against, so a referenced column that keeps its name but changes type
//! or nullability is classified `NeedsReview`, not `Stale`. Only a referenced
//! column *name* disappearing is a sound `Stale`.

use crate::contracts::view::ContractSchemaState;
use saya_store::StoredClaim;
use saya_types::{SchemaTree, Table};

/// Validity of `claim` against the live `schema` for its profile.
///
/// `schema == None` means the live schema could not be loaded for this profile.
pub(crate) fn schema_state_for(
    claim: &StoredClaim,
    schema: Option<&SchemaTree>,
) -> ContractSchemaState {
    let Some(schema) = schema else {
        return ContractSchemaState::LiveSchemaUnavailable;
    };
    // A fingerprint computed under a format the running binary no longer uses
    // cannot be compared — never claim Current for one. Re-running it under the
    // new format would launder an old record into a false match.
    if !claim.schema_fingerprint.is_current_format() {
        return ContractSchemaState::NeedsReview;
    }
    let Some(live_table) = live_table(schema, claim) else {
        return ContractSchemaState::Stale;
    };
    let live_fp = saya_types::SchemaFingerprint::of_table(claim.object.kind(), live_table);
    if live_fp == claim.schema_fingerprint {
        return ContractSchemaState::Current;
    }
    // Fingerprints differ. If any column the claim depends on is gone from the
    // live table, the claim lost something it referenced -> Stale. Otherwise the
    // change is in something the claim does not depend on -> NeedsReview.
    if referenced_column_missing(claim, live_table) {
        ContractSchemaState::Stale
    } else {
        ContractSchemaState::NeedsReview
    }
}

fn live_table<'s>(schema: &'s SchemaTree, claim: &StoredClaim) -> Option<&'s Table> {
    let obj = &claim.object;
    schema
        .databases
        .iter()
        .find(|db| db.name.eq_ignore_ascii_case(obj.catalog()))
        .and_then(|db| {
            db.schemas
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(obj.schema()))
        })
        .and_then(|s| {
            s.tables
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(obj.object()))
        })
}

fn referenced_column_missing(claim: &StoredClaim, live: &Table) -> bool {
    claim.referenced_columns.iter().any(|col| {
        !live
            .columns
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(col))
    })
}

#[cfg(test)]
mod unit {
    use super::*;
    use saya_types::{Column, Database, DatabaseObjectKind, Schema};

    fn schema_with(table: Table) -> SchemaTree {
        SchemaTree {
            databases: vec![Database {
                name: "catalog".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![table],
                }],
            }],
        }
    }

    #[test]
    fn version_check_precedes_object_lookup() {
        // An older-version fingerprint is NeedsReview even when the object is
        // present — the version check fires first, so the state is stable
        // regardless of the live table.
        let claim = StoredClaim {
            id: saya_types::ClaimId::parse("c-x").unwrap(),
            object: saya_types::DatabaseObjectRef::new(
                saya_types::ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap(),
                "catalog",
                "public",
                "orders",
                DatabaseObjectKind::Table,
            )
            .unwrap(),
            payload: None,
            origin: saya_types::ClaimOrigin::UserExplicit,
            status: saya_types::ClaimStatus::Confirmed,
            schema_fingerprint: saya_types::SchemaFingerprint::from_parts(
                saya_types::FINGERPRINT_VERSION.wrapping_add(1),
                &"a".repeat(64),
            )
            .unwrap(),
            referenced_columns: vec![],
            created_unix_ms: 0,
            updated_unix_ms: 0,
            last_verified_unix_ms: None,
        };
        let schema = schema_with(Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        });
        assert_eq!(
            schema_state_for(&claim, Some(&schema)),
            ContractSchemaState::NeedsReview
        );
    }
}
