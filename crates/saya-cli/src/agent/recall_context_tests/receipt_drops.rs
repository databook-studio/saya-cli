use super::*;

// P1a — recall receipt: what was supplied, and what bounds dropped.
//
// These assert on the `RecallReceipt` returned beside the blocks. The receipt
// names what recall **supplied** to the prompt (the claims whose rendered lines
// reached the block), never what the model **used** — see the type docs in
// `contracts/receipt.rs`. Nothing renders it yet.
// ===========================================================================

/// Spec test 1: a turn that supplies two claims produces a receipt naming
/// exactly those two, with ids matching what went into the prompt.
#[tokio::test]
async fn a_turn_supplying_two_claims_names_exactly_those_two_in_the_receipt() {
    let root = temp_root("p1a_two_claims");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let (_obj, time_id, alias_id) = seed_two_confirmed_claims(&store, &identity).await;
    let registry = registry_for("analytics", &identity);

    let (blocks, receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    let _ = &blocks; // the block still builds; this test is about the receipt.

    // One contract (one object) supplied, carrying exactly two claims.
    assert_eq!(
        receipt.supplied.len(),
        1,
        "one object was supplied: {:?}",
        receipt
    );
    let contract = &receipt.supplied[0];
    assert_eq!(contract.object, "catalog.public.orders");
    assert_eq!(contract.profile, "analytics", "the name, not the identity");
    assert_eq!(contract.claims.len(), 2, "both claims were supplied");

    // The ids match what went into the prompt, regardless of order.
    let mut supplied_ids: Vec<String> = contract
        .claims
        .iter()
        .map(|c| c.claim_id.to_string())
        .collect();
    supplied_ids.sort();
    let mut expected = vec![time_id.to_string(), alias_id.to_string()];
    expected.sort();
    assert_eq!(
        supplied_ids, expected,
        "the receipt names exactly the two supplied claim ids"
    );
    // The values are the short rendered forms the prompt shows, not payloads.
    assert!(
        contract.claims.iter().any(|c| c.value == "created_at"),
        "the default_time_column value is the column name: {:?}",
        contract.claims
    );
    assert!(
        contract.claims.iter().any(|c| c.value == "orders_alias"),
        "the table_alias value is the alias: {:?}",
        contract.claims
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec test 2 (count bound): objects beyond `max_objects` are dropped, and the
/// receipt's `dropped_by_bounds` counts the claims those objects carried. Each
/// object here has exactly one claim, so 7 objects under `max_objects=5` drops 2.
#[tokio::test]
async fn claims_dropped_by_the_object_count_bound_are_counted() {
    let root = temp_root("p1a_dropped_count");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let tree = many_orders_schema(&identity, 7);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    for i in 0..7 {
        let obj = object(&identity, &format!("orders{i}"));
        remember_confirmed_default_time_column(&store, &obj, &fp, "created_at").await;
    }
    let registry = registry_for("analytics", &identity);

    let (blocks, receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    let _ = &blocks;
    // 5 objects supplied, each with 1 claim → 5 supplied claims; 2 objects dropped.
    let supplied_claims: usize = receipt.supplied.iter().map(|c| c.claims.len()).sum();
    assert_eq!(supplied_claims, 5, "five objects' claims were supplied");
    assert_eq!(
        receipt.dropped_by_bounds, 2,
        "the two objects beyond max_objects are counted as dropped: {:?}",
        receipt
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec test 2 (per-object claim bound): claims beyond `max_claims_per_object`
/// within a kept object are dropped, and the receipt counts them. One object
/// with 20 claims under `max_claims_per_object=3` drops 17.
///
/// D-3 NOTE: a multi-valued slot holds at most 4 values, so 20 claims on one
/// object must span distinct slots. Seed 10 columns × {description, role} =
/// 20 distinct column-scoped items, all coexisting under D-3 cardinality.
#[tokio::test]
async fn claims_dropped_by_the_per_object_bound_are_counted() {
    use saya_types::ColumnRole;
    let root = temp_root("p1a_dropped_per_object");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");
    // A table with 10 columns so 10 distinct column-scoped slots exist.
    let cols: Vec<Column> = (0..10)
        .map(|i| Column {
            name: format!("c{i}"),
            data_type: if i % 2 == 0 {
                "timestamp".into()
            } else {
                "bigint".into()
            },
            nullable: false,
        })
        .collect();
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "orders".into(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: cols,
                }],
            }],
        }],
    };
    store.upsert_schema(identity.as_str(), &tree).await.unwrap();
    for i in 0..10 {
        let col = format!("c{i}");
        put_item(
            &store,
            &obj,
            ClaimPayload::column_description(&col, format!("desc {i}")).unwrap(),
            KnowledgeState::Active,
        )
        .await;
        let role = if i % 2 == 0 {
            ColumnRole::Timestamp
        } else {
            ColumnRole::Identifier
        };
        put_item(
            &store,
            &obj,
            ClaimPayload::column_role(&col, role, None).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    let registry = registry_for("analytics", &identity);
    let bounds = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 3,
        max_bytes: 16384,
    };
    let (blocks, receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        bounds,
        &registry,
        Some(&store),
    )
    .await;
    let _ = &blocks;
    // 3 of 20 supplied, 17 dropped by the per-object claim bound.
    assert_eq!(receipt.supplied.len(), 1);
    assert_eq!(receipt.supplied[0].claims.len(), 3);
    assert_eq!(
        receipt.dropped_by_bounds, 17,
        "the 17 claims beyond max_claims_per_object are counted: {:?}",
        receipt
    );
    let _ = fs::remove_dir_all(root);
}
