//! Read-time validity: a stored claim against the live schema.
//!
//! Phase 5b's typed drift rule. Classification only — reconciliation *writes*
//! are a later slice; a wrong rule that has already rewritten statuses is far
//! harder to undo than one that only reports. See the SPEC REVIEW in the
//! report for the two gaps found in spec-5b (the table's row order is a
//! condition list, not the rule's precedence; the version gate stays first).

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
    // cannot be compared — never claim Current for one. This gate fires first,
    // before the object is even looked up: the question is whether the digest
    // is *comparable*, not whether the object drifted. (The spec's rule table
    // lists this near the bottom; it is a condition list, not a precedence.)
    if !claim.schema_fingerprint.is_current_format() {
        return ContractSchemaState::NeedsReview;
    }
    let Some(live_table) = live_table(schema, claim) else {
        return ContractSchemaState::Stale;
    };
    if saya_types::SchemaFingerprint::of_table(claim.object.kind(), live_table)
        == claim.schema_fingerprint
    {
        return ContractSchemaState::Current;
    }
    // Fingerprints differ. Decide by what the claim actually depends on: a
    // referenced column that broke the claim reads Stale; one that only
    // narrowed reads NeedsReview; a drift entirely outside the claim's
    // columns reads NeedsReview too (the claim may still be true).
    match referenced_column_drift(claim, live_table) {
        Some(Drift::Stale) => ContractSchemaState::Stale,
        // NeedsReview covers both a narrowed referenced column and the
        // "all my columns match but the fingerprint moved" case (the drift is
        // in columns the claim does not depend on). Either way a human, not
        // the rule, must decide the claim is still sound.
        _ => ContractSchemaState::NeedsReview,
    }
}

/// How a single referenced column's snapshot relates to the live column.
#[derive(Debug, PartialEq, Eq)]
enum Drift {
    /// The column is gone, or it changed in a way that can silently break a
    /// claim built on the old shape: a different type, or it gained NULLs.
    Stale,
    /// The column narrowed — it became non-nullable when it was not. That
    /// only makes a previously-sometimes-null value always present, so the
    /// claim may still hold; a human should confirm.
    NeedsReview,
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

/// The worst verdict across the claim's referenced columns, or `None` when
/// every referenced column matches its snapshot. `None` is *not* Current —
/// the fingerprint already differed, so a caller that returns here must fall
/// through to NeedsReview: the drift is in something the claim does not
/// depend on.
///
/// A claim with no referenced columns returns `None`: nothing it depends on
/// can have broken, so it is NeedsReview (never Stale) when the fingerprint
/// moves.
fn referenced_column_drift(claim: &StoredClaim, live: &Table) -> Option<Drift> {
    let mut worst: Option<Drift> = None;
    for snap in &claim.referenced_columns {
        let Some(col) = live
            .columns
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(&snap.name))
        else {
            // Absent by name — renamed or dropped. The claim lost something it
            // referenced.
            return Some(Drift::Stale);
        };
        // An unknown snapshot (empty type) can prove nothing: never match, so
        // a pre-5a row with a moved fingerprint never reads Current. We cannot
        // distinguish "retyped" from "unchanged" without a stored type, so
        // fall through to NeedsReview rather than guessing either way.
        if snap.data_type.trim().is_empty() {
            worst = worst.or(Some(Drift::NeedsReview));
            continue;
        }
        // Exact string equality after trimming. No cross-dialect normalisation:
        // `int4` and `integer` may or may not coincide by backend, and a
        // permissive guess keeps a stale claim alive silently — the exact
        // failure this rule exists to prevent. The same discovery path writes
        // both the snapshot and the live type, so the round-trip it actually
        // compares is exact-by-construction; this only refuses to guess across
        // *different* backends, which is the safe refusal.
        if col.data_type.trim() != snap.data_type.trim() {
            return Some(Drift::Stale);
        }
        // Nullable-ward is Stale: a "default time column" that is now sometimes
        // null breaks the queries the claim exists to shape. Non-nullable-ward
        // (lost nullability) only narrows what was already true -> NeedsReview.
        if col.nullable && !snap.nullable {
            return Some(Drift::Stale);
        }
        if !col.nullable && snap.nullable {
            worst = worst.or(Some(Drift::NeedsReview));
        }
    }
    worst
}

#[cfg(test)]
mod unit {
    use super::*;
    use saya_types::{Column, DatabaseObjectKind, SchemaFingerprint};

    fn schema_with(table: Table) -> SchemaTree {
        SchemaTree {
            databases: vec![saya_types::Database {
                name: "catalog".into(),
                schemas: vec![saya_types::Schema {
                    name: "public".into(),
                    tables: vec![table],
                }],
            }],
        }
    }

    fn base_claim() -> StoredClaim {
        StoredClaim {
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
            schema_fingerprint: SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap(),
            referenced_columns: vec![],
            created_unix_ms: 0,
            updated_unix_ms: 0,
            last_verified_unix_ms: None,
        }
    }

    #[test]
    fn version_check_precedes_object_lookup() {
        // An older-version fingerprint is NeedsReview even when the object is
        // present — the version check fires first, so the state is stable
        // regardless of the live table.
        let mut claim = base_claim();
        claim.schema_fingerprint = SchemaFingerprint::from_parts(2, &"a".repeat(64)).unwrap();
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

    #[test]
    fn no_referenced_columns_with_moved_fingerprint_is_needs_review() {
        // A table-level claim depends on nothing: a moved fingerprint cannot
        // have broken any column it references, so it is NeedsReview, not
        // Stale. The live digest differs from the stored one by construction.
        let mut claim = base_claim();
        let table = Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        };
        claim.schema_fingerprint = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
        // Add a column to move the fingerprint without touching the claim.
        let drifted = Table {
            name: "orders".into(),
            columns: vec![
                Column {
                    name: "id".into(),
                    data_type: "bigint".into(),
                    nullable: false,
                },
                Column {
                    name: "note".into(),
                    data_type: "text".into(),
                    nullable: true,
                },
            ],
        };
        assert_eq!(
            referenced_column_drift(&claim, &drifted),
            None,
            "a claim with no referenced columns has no drift"
        );
        // And the public rule turns that None plus a moved fingerprint into
        // NeedsReview, never Stale.
        let live = schema_with(drifted);
        assert_eq!(
            schema_state_for(&claim, Some(&live)),
            ContractSchemaState::NeedsReview
        );
    }
}
