//! Read-time validity: a stored claim against the live schema.
//!
//! Phase 5b's typed drift rule. Classification only — reconciliation *writes*
//! are a later slice; a wrong rule that has already rewritten statuses is far
//! harder to undo than one that only reports. See the SPEC REVIEW in the
//! report for the two gaps found in spec-5b (the table's row order is a
//! condition list, not the rule's precedence; the version gate stays first).

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::view::ContractSchemaState;
use saya_store::StoredClaim;
use saya_types::{SchemaTree, Table};

/// Validity of `claim` against the schema known for its profile.
///
/// `availability` is the three-state schema input (see [`SchemaAvailability`]):
/// `Missing` and `Unavailable` both classify [`ContractSchemaState::LiveSchemaUnavailable`],
/// never `Stale` — an absent or unreadable cache is not evidence a column is
/// gone. `freshness` gates currency on the model path: a cached schema older
/// than the bound cannot classify a claim as `Current`, so it too reads
/// `LiveSchemaUnavailable`. Human-review paths pass [`SchemaFreshness::Unbounded`]
/// to use the cache regardless of age.
pub(crate) fn schema_state_for(
    claim: &StoredClaim,
    availability: &SchemaAvailability,
    freshness: SchemaFreshness,
) -> ContractSchemaState {
    // Missing, Unavailable, and a too-old Available all hand back no schema —
    // the same honest "we cannot classify" verdict, never a fabricated tree that
    // would read Stale (the P1 bug) or Current.
    let Some(schema) = availability.live_table_schema(freshness) else {
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
    schema.find_table(obj.catalog(), obj.schema(), obj.object())
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
    use crate::contracts::availability::SchemaFreshness;
    use saya_types::{Column, DatabaseObjectKind, SchemaFingerprint};

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
            schema_state_for(
                &claim,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW)
            ),
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
            schema_state_for(
                &claim,
                &SchemaAvailability::available(live, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW)
            ),
            ContractSchemaState::NeedsReview
        );
    }

    // -----------------------------------------------------------------
    // P1 regression: a store error must not collapse to an empty schema.
    //
    // The bug (recall_context `resolve_profiles`) turned both "no cached
    // schema" and "the store returned an error" into `SchemaTree::default()`,
    // passed as `Some`, so validity saw a schema that exists and lacked the
    // object → `Stale`. Since stale contracts are excluded from the model, a
    // store hiccup silently muted every claim. `Unavailable` and `Missing`
    // must both read `LiveSchemaUnavailable`, and `Unavailable` must stay
    // distinct from `Missing` so a diagnostic can name which it was.
    // -----------------------------------------------------------------

    fn current_claim_against(table: Table) -> StoredClaim {
        let mut claim = base_claim();
        claim.schema_fingerprint = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
        claim
    }

    #[test]
    fn store_error_yields_live_schema_unavailable_not_stale() {
        // A store error (`Unavailable`) must never read as `Stale`. Before the
        // fix the collapsed empty tree made every claim Stale; now the verdict
        // is the honest "we could not look", so no claim is excluded as stale.
        let table = Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        };
        let claim = current_claim_against(table);
        assert_eq!(
            schema_state_for(
                &claim,
                &SchemaAvailability::Unavailable,
                SchemaFreshness::for_model(FRESH_NOW)
            ),
            ContractSchemaState::LiveSchemaUnavailable,
            "a store error is not evidence a column is gone"
        );
    }

    #[test]
    fn missing_cache_entry_yields_live_schema_unavailable_not_stale() {
        // Nothing discovered yet (`Missing`) reads the same as a store error
        // for classification, but stays a distinct variant so a diagnostic can
        // say "run `connection schema` to discover" vs "the store was
        // unreadable".
        let table = Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        };
        let claim = current_claim_against(table);
        assert_eq!(
            schema_state_for(
                &claim,
                &SchemaAvailability::Missing,
                SchemaFreshness::for_model(FRESH_NOW)
            ),
            ContractSchemaState::LiveSchemaUnavailable,
            "a missing cache is not evidence a column is gone"
        );
    }

    #[test]
    fn absent_object_with_a_real_cached_schema_still_yields_stale() {
        // The fix must not blunt real staleness: a genuine schema that simply
        // lacks the claim's object is `Stale`, the column is actually gone.
        let claim = current_claim_against(Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        });
        // A real cached schema with a *different* table — `orders` is absent.
        let schema = schema_with(Table {
            name: "other".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        });
        assert_eq!(
            schema_state_for(
                &claim,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW)
            ),
            ContractSchemaState::Stale,
            "a real schema lacking the object is genuine staleness"
        );
    }

    #[test]
    fn fresh_cache_classifies_current() {
        // A cache within the freshness bound matching the claim's fingerprint
        // reads `Current` — the fix preserves the happy path.
        let table = Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        };
        let claim = current_claim_against(table.clone());
        let schema = schema_with(table);
        assert_eq!(
            schema_state_for(
                &claim,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW)
            ),
            ContractSchemaState::Current
        );
    }

    #[test]
    fn cache_older_than_the_bound_classifies_live_schema_unavailable_on_model_path() {
        // A cache older than the bound must not classify `Current` on the model
        // path — we cannot vouch for currency. It reads `LiveSchemaUnavailable`
        // (the claim still reaches the model, labelled, not silently current).
        let table = Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        };
        let claim = current_claim_against(table.clone());
        // Observed at 0; "now" is 25h later — past the 24h bound.
        let now = 25 * 60 * 60 * 1000;
        let schema = schema_with(table);
        assert_eq!(
            schema_state_for(
                &claim,
                &SchemaAvailability::available(schema, 0),
                SchemaFreshness::for_model(now)
            ),
            ContractSchemaState::LiveSchemaUnavailable,
            "a stale-by-age cache cannot vouch for currency on the model path"
        );
    }

    #[test]
    fn cache_older_than_the_bound_still_classifies_on_the_human_path() {
        // The human-review path is unbounded: the same 25h-old cache classifies
        // normally, because a reviewer is not asked to trust a query built on
        // their own contracts. `contracts list`/`show`/`queue` rely on this.
        let table = Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        };
        let claim = current_claim_against(table.clone());
        let schema = schema_with(table);
        assert_eq!(
            schema_state_for(
                &claim,
                &SchemaAvailability::available(schema, 0),
                SchemaFreshness::Unbounded
            ),
            ContractSchemaState::Current,
            "human path uses a stale-by-age cache to show what it knows"
        );
    }
}
