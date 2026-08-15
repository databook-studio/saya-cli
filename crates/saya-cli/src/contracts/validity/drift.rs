//! The per-column drift rule: how a stored claim's referenced-column
//! snapshots relate to the columns of a live table.
//!
//! Pure comparison logic, separate from the availability/version gating in
//! [`super`]. A claim's fingerprint already differed by the time this runs —
//! the rule decides whether a column the claim *depends on* broke it (`Stale`)
//! or whether the drift is something the claim can survive (`NeedsReview`).

use saya_store::StoredClaim;
use saya_types::Table;

/// How a single referenced column's snapshot relates to the live column.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Drift {
    /// The column is gone, or it changed in a way that can silently break a
    /// claim built on the old shape: a different type, or it gained NULLs.
    Stale,
    /// The column narrowed — it became non-nullable when it was not. That
    /// only makes a previously-sometimes-null value always present, so the
    /// claim may still hold; a human should confirm.
    NeedsReview,
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
pub(super) fn referenced_column_drift(claim: &StoredClaim, live: &Table) -> Option<Drift> {
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
mod tests {
    use super::*;
    use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
    use crate::contracts::validity::schema_state_for;
    use crate::contracts::view::ContractSchemaState;
    use saya_types::{Column, DatabaseObjectKind, SchemaFingerprint, SchemaTree};

    /// A cache observed at "now" so the model-path freshness bound never fires —
    /// these tests exercise the table/fingerprint logic, not the age gate.
    const FRESH_NOW: i64 = 1_000_000;

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
            schema_state_for(
                &claim,
                &SchemaAvailability::available(live, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW)
            ),
            ContractSchemaState::NeedsReview
        );
    }
}
