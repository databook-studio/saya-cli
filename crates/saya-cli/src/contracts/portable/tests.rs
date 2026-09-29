//! Unit tests for the portable mapping: resolution, ambiguity, the
//! relationship→join-rule filing, and the export document shape. The full
//! command flows live in `tests/contracts_portable.rs`.

use super::map_items;
use crate::contracts::portable::{PortableError, export, import_document, read_context};
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SqliteStateStore};
use saya_types::{
    CONTEXT_FORMAT, CONTEXT_FORMAT_VERSION, ClaimOrigin, ClaimPayload, Column, ContextError,
    ContextItem, Database, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot, KnowledgeState,
    MAX_DOCUMENT_BYTES, PortableObject, PortablePayload, ProfileIdentity, Schema, SchemaBinding,
    SchemaFingerprint, SchemaTree, Table,
};

fn identity() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

/// `(schema, table name, columns)` triples of one schema under one catalog.
type Tables<'a> = &'a [(&'a str, &'a str, &'a [(&'a str, &'a str)])];

fn tree(tables: Tables<'_>) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: tables
                    .iter()
                    .map(|(_schema, name, columns)| Table {
                        name: (*name).into(),
                        columns: columns
                            .iter()
                            .map(|(name, data_type)| Column {
                                name: (*name).into(),
                                data_type: (*data_type).into(),
                                nullable: true,
                            })
                            .collect(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                    })
                    .collect(),
            }],
        }],
    }
}

fn item(name: &str, payload: &ClaimPayload) -> ContextItem {
    ContextItem {
        object: PortableObject {
            catalog: Some("analytics".into()),
            schema: Some("public".into()),
            name: name.into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(payload).unwrap(),
        origin_note: Some("dbt 1.8.6 model.jaffle.orders".into()),
    }
}

#[test]
fn resolution_is_case_insensitive_and_ambiguous_when_several_match() {
    let schema = tree(&[("analytics.public", "orders", &[("id", "bigint")])]);
    let items = vec![item(
        "ORDERS",
        &ClaimPayload::table_description("One row per order.").unwrap(),
    )];
    let mapped = map_items(&items, &schema, &identity());
    assert_eq!(mapped.unavailable.len(), 0);
    assert_eq!(mapped.planned[0].label, "analytics.public.orders");

    // Two same-spelled tables in different schemas under one catalog: with the
    // schema named the resolution is exact, without it the spelling never
    // picks and the item is unavailable.
    let second_schema = Schema {
        name: "private".into(),
        tables: schema.databases[0].schemas[0].tables.clone(),
    };
    let doubled = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![schema.databases[0].schemas[0].clone(), second_schema],
        }],
    };
    let ambiguous = vec![ContextItem {
        object: PortableObject {
            catalog: Some("analytics".into()),
            schema: None,
            name: "orders".into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(
            &ClaimPayload::table_description("One row per order.").unwrap(),
        )
        .unwrap(),
        origin_note: None,
    }];
    let mapped = map_items(&ambiguous, &doubled, &identity());
    assert_eq!(mapped.planned.len(), 0);
    assert_eq!(
        mapped.unavailable[0].reason,
        "the name matches several objects in this profile's schema"
    );

    // The schema named, the same document resolves again.
    let named = vec![ContextItem {
        object: PortableObject {
            catalog: Some("analytics".into()),
            schema: Some("public".into()),
            name: "orders".into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(&ClaimPayload::table_alias("customers").unwrap())
            .unwrap(),
        origin_note: None,
    }];
    let mapped = map_items(&named, &doubled, &identity());
    assert_eq!(mapped.planned.len(), 1);
}

#[test]
fn a_relationship_files_as_its_keyed_join_rule() {
    let schema = tree(&[
        ("analytics.public", "orders", &[("customer_id", "bigint")]),
        ("analytics.public", "customers", &[("id", "bigint")]),
    ]);
    // Build the portable relationship through the validating constructors: the
    // synthetic profile stands in for the identity the constructor requires.
    let profile = identity();
    let target = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "customers",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let claim = ClaimPayload::relationship(
        target,
        vec!["customer_id".into()],
        vec!["id".into()],
        saya_types::Cardinality::ManyToOne,
    )
    .unwrap();
    let items = vec![ContextItem {
        object: PortableObject {
            catalog: Some("analytics".into()),
            schema: Some("public".into()),
            name: "orders".into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(&claim).unwrap(),
        origin_note: None,
    }];
    let mapped = map_items(&items, &schema, &identity());
    assert_eq!(mapped.unavailable.len(), 0, "{:?}", mapped.unavailable);
    let planned = &mapped.planned[0];
    assert_eq!(planned.slot, KnowledgeSlot::RelationJoinRule.as_str());
    let ClaimPayload::JoinRule {
        target,
        local_columns,
        target_columns,
        condition,
        ..
    } = &planned.item.value
    else {
        panic!("expected a filed join rule");
    };
    assert_eq!(target, "analytics.public.customers");
    assert_eq!(local_columns, &["customer_id".to_string()]);
    assert_eq!(target_columns, &["id".to_string()]);
    assert_eq!(condition, "customer_id = id");
    // The binding rides the local keys, so the fact invalidates when the key
    // column disappears.
    assert!(planned.item.schema_binding_json.contains("customer_id"));
}

/// The mapping's credential pre-check mirrors the store's batch admission: an
/// item whose serialised payload the redactor would rewrite is refused in the
/// mapping — reported, never offered to the batch — while the clean items
/// beside it stay planned.
#[test]
fn a_credential_shaped_claim_is_unavailable_not_planned() {
    let schema = tree(&[
        ("analytics.public", "orders", &[("id", "bigint")]),
        ("analytics.public", "customers", &[("id", "bigint")]),
    ]);
    let items = vec![
        item(
            "orders",
            &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
        ),
        item("orders", &ClaimPayload::table_alias("customers").unwrap()),
        item(
            "customers",
            &ClaimPayload::table_description("The ingest box runs with password=hunter2.").unwrap(),
        ),
    ];
    let mapped = map_items(&items, &schema, &identity());
    assert_eq!(mapped.planned.len(), 2, "{:?}", mapped.unavailable);
    assert_eq!(mapped.unavailable.len(), 1, "{:?}", mapped.unavailable);
    assert_eq!(mapped.unavailable[0].label, "analytics.public.customers");
    assert_eq!(mapped.unavailable[0].reason, "credential-shaped text");
}

#[tokio::test]
async fn export_and_import_round_trip_through_the_file() {
    let root = std::env::temp_dir().join(format!(
        "saya-portable-roundtrip-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let profile = identity();
    let object = DatabaseObjectRef::new(
        profile.clone(),
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("customers").unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).unwrap();
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload.clone(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: SchemaFingerprint::from_parts(1, &"0".repeat(64)).unwrap(),
        })
        .await
        .unwrap();

    let path = root.join("context.json");
    let outcome = export(&store, &profile, &path, false).await.unwrap();
    assert_eq!(outcome.total, 1);
    assert_eq!(outcome.kinds, vec![("table_alias".to_string(), 1)]);

    // The file parses, and the imported claim files under a fresh profile's
    // identity as a Pending team-file row.
    let document = read_context(&path).unwrap();
    assert_eq!(document.format, CONTEXT_FORMAT);
    assert_eq!(document.version, CONTEXT_FORMAT_VERSION);
    let fresh = ProfileIdentity::parse(&format!("p-{}", "b".repeat(64))).unwrap();
    let outcome = import_document(
        &store,
        &fresh,
        &document.items,
        &SchemaTree::default(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(outcome.total(), 1);
    assert_eq!(
        outcome.unavailable.len(),
        1,
        "the empty tree resolves nothing"
    );

    // An oversize read is refused before parsing; the cap is exact.
    let oversize = root.join("oversize.json");
    std::fs::File::create(&oversize)
        .unwrap()
        .set_len(MAX_DOCUMENT_BYTES as u64 + 1)
        .unwrap();
    assert!(matches!(
        read_context(&oversize),
        Err(PortableError::Document(ContextError::Oversize(_)))
    ));

    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
