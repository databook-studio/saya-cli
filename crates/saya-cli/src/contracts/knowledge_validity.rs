//! Spec D-2 — one computed validity over the new state vocabulary.
//!
//! `KnowledgeValidity` is the verdict a knowledge item earns against the schema
//! known for its profile. The legacy `StoredClaim`-based classifier
//! (`schema_state_for` over `contract_claims`) and its `ClaimStatus`→`KnowledgeState`
//! mapping were deleted in Chunk 5 with the legacy store they read; the
//! knowledge path is the only path, and [`item_validity_for`] is the verdict it
//! uses. [`KnowledgeValidity`] survives because the render/retrieval layers
//! still speak [`crate::contracts::view::ContractSchemaState`], and the
//! `From<KnowledgeValidity> for ContractSchemaState` impl is the bridge.
//!
//! Why a separate enum at all: `ContractSchemaState::Stale` is *computed*, but
//! the legacy `ClaimStatus::Stale` was *persisted* — the same fact had two homes
//! and they could disagree. D-1 introduced [`KnowledgeState`] with no `Stale`
//! variant because staleness is a read-time computation, not a stored state.
//! [`item_validity_for`] owns that computation: `Invalid` is what a gone bound
//! column reads as, and it is never persisted.
//!
//! [`Dismissed`]: saya_types::KnowledgeState::Dismissed

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::view::ContractSchemaState;
use saya_store::KnowledgeItem;
use saya_types::{FINGERPRINT_VERSION, KnowledgeState, SchemaBinding};

/// The computed validity of a claim against the schema known for its profile.
///
/// Ordered worst-first for aggregation via [`Self::aggregate`]:
/// `Invalid` beats `SchemaUnavailable` beats `NeedsReview` beats `Valid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnowledgeValidity {
    /// The claim matches the schema it was made against (← `Current`).
    Valid,
    /// The schema moved in a way a human must judge (← `NeedsReview`). Not
    /// dropped on the model path: one unrelated column added to a wide table
    /// must not mute every claim on it.
    NeedsReview,
    /// No usable schema: the cache is missing, unreadable, or (on the model
    /// path) past the freshness bound (← `LiveSchemaUnavailable`). Never
    /// `Invalid` — an absent cache is not evidence a column is gone.
    SchemaUnavailable,
    /// A referenced column the claim depends on is gone, or changed in a way
    /// that can silently break it (← `Stale`). A contract that aggregates to
    /// `Invalid` is not safe for the model to read as a fact.
    Invalid,
}

impl KnowledgeValidity {
    /// A claim cannot be more reassuring than the worst claim in the same
    /// contract — one `Invalid` claim must flag the object, not hide behind a
    /// `Valid` sibling. Mirrors [`ContractSchemaState::aggregate`].
    pub(crate) fn aggregate(self, other: Self) -> Self {
        let rank = |state: Self| match state {
            Self::Invalid => 3,
            Self::SchemaUnavailable => 2,
            Self::NeedsReview => 1,
            Self::Valid => 0,
        };
        if rank(self) >= rank(other) {
            self
        } else {
            other
        }
    }
}

/// Bridge to the render/retrieval vocabulary. [`assemble`] and [`show`] compute
/// a [`KnowledgeValidity`] per item and fold it into the contract's
/// [`ContractSchemaState`] via this `into()`, so the render and receipt layers
/// keep speaking the four-state vocabulary they were built on.
///
/// [`assemble`]: super::assemble
/// [`show`]: super::show
impl From<KnowledgeValidity> for ContractSchemaState {
    fn from(validity: KnowledgeValidity) -> Self {
        match validity {
            KnowledgeValidity::Valid => Self::Current,
            KnowledgeValidity::NeedsReview => Self::NeedsReview,
            KnowledgeValidity::SchemaUnavailable => Self::LiveSchemaUnavailable,
            KnowledgeValidity::Invalid => Self::Stale,
        }
    }
}

/// The computed validity of a [`KnowledgeItem`] under the D-4 binding model.
///
/// The item carries its structural dependency as a serialised [`SchemaBinding`]
/// (`Table` | `Column { column, requirement }`) plus a `fingerprint_version`.
/// D-4's contract is that a fact depends only on what it names, so validity is
/// [`SchemaBinding::validate_in_tree`] against the live schema — an unrelated
/// column changing never invalidates (the false-alarm "needs review" the old
/// whole-table fingerprint model produced is gone by design; see
/// [`SchemaBinding`]). The four-state [`KnowledgeValidity`] is produced by the
/// gates that wrap the binding check:
///
/// - [`KnowledgeState::Dismissed`] → [`Invalid`](KnowledgeValidity::Invalid)
///   without a schema lookup. A dismissed item is withdrawn from use; no
///   revalidation revives it, and it must never read `Valid`. This fires
///   before the schema lookup so a missing or unreadable cache cannot upgrade
///   it to `SchemaUnavailable` (which is kept on the model path).
/// - No live schema (`Missing`, `Unavailable`, or — on the model path — a
///   cached schema older than the freshness bound) →
///   [`SchemaUnavailable`](KnowledgeValidity::SchemaUnavailable). An absent or
///   unreadable cache is not evidence a bound column is gone.
/// - A `fingerprint_version` this build did not write →
///   [`NeedsReview`](KnowledgeValidity::NeedsReview). The binding was derived
///   under a format this build cannot faithfully compare, so the item is held
///   for human review rather than trusted or silently dropped.
/// - A binding JSON this build cannot deserialize (a row written by an
///   incompatible build) → `NeedsReview`, for the same reason.
/// - Otherwise the deserialised binding is validated against the live tree:
///   `Valid` if every dependency it names is satisfied, `Invalid` if a bound
///   column is gone or lost its required semantic type.
///
/// Pure: no store, no clock, no I/O. `freshness` is an input so the model path
/// can pass a controlled stamp and the human path can pass `Unbounded`.
pub(crate) fn item_validity_for(
    item: &KnowledgeItem,
    availability: &SchemaAvailability,
    freshness: SchemaFreshness,
) -> KnowledgeValidity {
    if matches!(item.state, KnowledgeState::Dismissed) {
        return KnowledgeValidity::Invalid;
    }
    let Some(schema) = availability.live_table_schema(freshness) else {
        return KnowledgeValidity::SchemaUnavailable;
    };
    if item.fingerprint_version != FINGERPRINT_VERSION {
        return KnowledgeValidity::NeedsReview;
    }
    match serde_json::from_str::<SchemaBinding>(&item.schema_binding_json) {
        Ok(binding) => {
            let obj = &item.object;
            match binding.validate_in_tree(schema, obj.catalog(), obj.schema(), obj.object()) {
                saya_types::BindingValidity::Valid => KnowledgeValidity::Valid,
                saya_types::BindingValidity::Invalid => KnowledgeValidity::Invalid,
            }
        }
        // A binding this build cannot interpret was written by an incompatible
        // build. Hold the item for human review rather than dropping a
        // possibly-valid fact or trusting an uninterpretable one.
        Err(_) => KnowledgeValidity::NeedsReview,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
    use saya_types::{Column, DatabaseObjectKind, SchemaTree};

    /// A cache observed at "now" so the model-path freshness bound never fires —
    /// these tests exercise the table/fingerprint logic, not the age gate.
    const FRESH_NOW: i64 = 1_000_000;

    fn schema_with(table: saya_types::Table) -> SchemaTree {
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

    fn table(cols: &[(&str, &str, bool)]) -> saya_types::Table {
        saya_types::Table {
            name: "orders".into(),
            columns: cols
                .iter()
                .map(|(name, ty, nullable)| Column {
                    name: (*name).into(),
                    data_type: (*ty).into(),
                    nullable: *nullable,
                })
                .collect(),
        }
    }

    // Aggregation: one Invalid among Valid siblings yields Invalid. This is the
    // one `knowledge_validity_for`-independent guarantee that survived the
    // Chunk 5 prune — `aggregate` is still the fold `assemble`/`show` use.
    #[test]
    fn aggregate_one_invalid_among_valid_is_invalid() {
        let verdicts = [
            KnowledgeValidity::Valid,
            KnowledgeValidity::Invalid,
            KnowledgeValidity::Valid,
        ];
        let aggregate = verdicts
            .iter()
            .copied()
            .fold(KnowledgeValidity::Valid, KnowledgeValidity::aggregate);
        assert_eq!(
            aggregate,
            KnowledgeValidity::Invalid,
            "one invalid claim flags its object; it cannot hide behind a valid sibling"
        );
    }

    // -----------------------------------------------------------------
    // item_validity_for: the D-4 read-time verdict over a KnowledgeItem.
    //
    // The item carries its structural dependency as a serialised
    // `SchemaBinding` (Table | Column{column, requirement}) plus a
    // `fingerprint_version`. Validity is `SchemaBinding::validate_in_tree` — a
    // fact depends only on what it names, so an unrelated column changing
    // never invalidates (D-4: stop crying wolf). The four-state vocabulary is
    // produced by the availability/freshness gate (SchemaUnavailable), the
    // Dismissed state (Invalid), and a non-current fingerprint version
    // (NeedsReview — a binding derived under a format this build cannot
    // compare, held for human review rather than trusted or silently dropped).
    // -----------------------------------------------------------------

    use crate::contracts::availability::MODEL_SCHEMA_MAX_AGE_MS;
    use saya_store::KnowledgeItem;
    use saya_types::{
        ClaimOrigin, ColumnRequirement, FINGERPRINT_VERSION, KnowledgeSlot, SchemaBinding,
    };

    fn item_profile() -> saya_types::ProfileIdentity {
        saya_types::ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
    }

    fn item_object() -> saya_types::DatabaseObjectRef {
        saya_types::DatabaseObjectRef::new(
            item_profile(),
            "catalog",
            "public",
            "orders",
            DatabaseObjectKind::Table,
        )
        .unwrap()
    }

    /// A knowledge item for a `default_time_column` on `created_at`, written
    /// under the current fingerprint version. `binding_json` is the serialised
    /// `SchemaBinding` the ingest path would have stored.
    fn time_column_item(state: KnowledgeState, binding_json: &str) -> KnowledgeItem {
        KnowledgeItem {
            id: "ki-1".into(),
            object: item_object(),
            slot: KnowledgeSlot::TableDefaultTime,
            cardinality_single: true,
            value: saya_types::ClaimPayload::default_time_column("created_at").unwrap(),
            source: ClaimOrigin::UserExplicit,
            state,
            schema_binding_json: binding_json.into(),
            fingerprint_version: FINGERPRINT_VERSION,
            created_unix_ms: 0,
            updated_unix_ms: 0,
        }
    }

    fn time_binding() -> String {
        serde_json::to_string(&SchemaBinding::Column {
            column: "created_at".to_string(),
            requirement: ColumnRequirement::Time,
        })
        .unwrap()
    }

    fn table_binding() -> String {
        serde_json::to_string(&SchemaBinding::Table).unwrap()
    }

    // 1. A live schema whose table carries the bound column at the right type
    //    classifies Valid — the happy path under D-4.
    #[test]
    fn item_validity_for_a_satisfied_binding_is_valid() {
        let item = time_column_item(KnowledgeState::Active, &time_binding());
        let schema = schema_with(table(&[("created_at", "timestamp", false)]));
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Valid,
            "a binding the live schema satisfies is valid"
        );
    }

    // 2. An unrelated column added to the table does NOT invalidate the
    //    binding (D-4's whole point). Under the old whole-table fingerprint
    //    model this read NeedsReview; under D-4 it reads Valid.
    #[test]
    fn item_validity_unrelated_column_change_is_valid_under_d4() {
        let item = time_column_item(KnowledgeState::Active, &time_binding());
        let schema = schema_with(table(&[
            ("id", "bigint", false),
            ("created_at", "timestamp", false),
            ("note", "text", true),
        ]));
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Valid,
            "an unrelated column changing must not invalidate under D-4"
        );
    }

    // 3. A dropped referenced column → Invalid (the gone-column case, D-4).
    #[test]
    fn item_validity_dropped_bound_column_is_invalid() {
        let item = time_column_item(KnowledgeState::Active, &time_binding());
        let schema = schema_with(table(&[("id", "bigint", false)]));
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Invalid,
            "a gone bound column makes the item invalid"
        );
    }

    // 4. A bound column retyped away from temporal → Invalid.
    #[test]
    fn item_validity_retyped_bound_column_is_invalid() {
        let item = time_column_item(KnowledgeState::Active, &time_binding());
        let schema = schema_with(table(&[("created_at", "text", false)]));
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Invalid,
            "a bound column that lost its temporal type is invalid"
        );
    }

    // 5. A missing or unavailable schema → SchemaUnavailable, never Invalid.
    #[test]
    fn item_validity_missing_or_unavailable_schema_is_schema_unavailable() {
        let item = time_column_item(KnowledgeState::Active, &time_binding());
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::Missing,
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::SchemaUnavailable,
            "a missing cache is not evidence a bound column is gone"
        );
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::Unavailable,
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::SchemaUnavailable,
            "a store error is not evidence a bound column is gone"
        );
    }

    // 6. A stale-by-age cached schema beyond the model-path freshness bound →
    //    SchemaUnavailable (kept, labelled), not Valid.
    #[test]
    fn item_validity_stale_by_age_cache_is_schema_unavailable() {
        let item = time_column_item(KnowledgeState::Active, &time_binding());
        let schema = schema_with(table(&[("created_at", "timestamp", false)]));
        let now = MODEL_SCHEMA_MAX_AGE_MS + 1;
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(schema, 0),
                SchemaFreshness::for_model(now),
            ),
            KnowledgeValidity::SchemaUnavailable,
            "a stale-by-age cache cannot vouch for currency on the model path"
        );
    }

    // 7. A non-current fingerprint version → NeedsReview: the binding was
    //    derived under a format this build cannot faithfully compare, so the
    //    item is held for human review rather than trusted or silently dropped.
    #[test]
    fn item_validity_non_current_fingerprint_version_is_needs_review() {
        let mut item = time_column_item(KnowledgeState::Active, &time_binding());
        item.fingerprint_version = FINGERPRINT_VERSION + 1;
        let schema = schema_with(table(&[("created_at", "timestamp", false)]));
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::NeedsReview,
            "a binding written under an unknown format is held for review"
        );
    }

    // 8. A Dismissed item is Invalid without consulting the schema, and stays
    //    Invalid when the schema is missing too.
    #[test]
    fn item_validity_dismissed_is_invalid_regardless_of_schema() {
        let item = time_column_item(KnowledgeState::Dismissed, &time_binding());
        let schema = schema_with(table(&[("created_at", "timestamp", false)]));
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Invalid,
            "a dismissed item is not a live fact"
        );
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::Missing,
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Invalid,
            "dismissal beats schema unavailability"
        );
    }

    // 9. A Table binding classifies Valid whenever the table exists, and
    //    Invalid when it is gone — independent of any column.
    #[test]
    fn item_validity_table_binding_depends_only_on_the_table_existing() {
        let item = KnowledgeItem {
            id: "ki-2".into(),
            object: item_object(),
            slot: KnowledgeSlot::TableGrain,
            cardinality_single: true,
            value: saya_types::ClaimPayload::table_grain("one row per order").unwrap(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: table_binding(),
            fingerprint_version: FINGERPRINT_VERSION,
            created_unix_ms: 0,
            updated_unix_ms: 0,
        };
        let present = schema_with(table(&[("id", "bigint", false)]));
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(present, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Valid,
            "a table binding is valid when the table exists"
        );
        let absent_table = schema_with(saya_types::Table {
            name: "other".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
        });
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(absent_table, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Invalid,
            "a table binding is invalid when the table is gone"
        );
    }

    // 10. A binding JSON this build cannot deserialize (a row written by an
    //     incompatible build) is held for review, not silently dropped.
    #[test]
    fn item_validity_unparseable_binding_is_needs_review() {
        let item = time_column_item(KnowledgeState::Active, "not json at all");
        let schema = schema_with(table(&[("created_at", "timestamp", false)]));
        assert_eq!(
            item_validity_for(
                &item,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::NeedsReview,
            "an uninterpretable binding is held for human review, not dropped"
        );
    }
}
