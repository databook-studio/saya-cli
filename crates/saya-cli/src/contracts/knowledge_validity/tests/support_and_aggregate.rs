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
        primary_key: vec![],
        foreign_keys: vec![],
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
        value: saya_types::ClaimPayload::default_time_column("created_at", None).unwrap(),
        source: ClaimOrigin::UserExplicit,
        state,
        cleanup: saya_store::CleanupState::Complete,
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
