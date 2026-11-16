/// A claim the store's admission predicate refuses (an admission shape, not a
/// credential marker) aborts the whole import — preview and apply alike —
/// with the same typed error naming the object, never the claim text, and
/// nothing written, not even the clean items beside it.
#[tokio::test]
async fn a_store_refused_item_aborts_preview_and_apply_alike() {
    let root = std::env::temp_dir().join(format!(
        "saya-portable-refusal-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
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
            &ClaimPayload::table_description("Select the latest row from each customer.").unwrap(),
        ),
    ];
    for preview in [true, false] {
        let outcome = import_document(&store, &identity(), &items, &schema, preview).await;
        let Err(error) = &outcome else {
            panic!("preview={preview} must refuse, not land items");
        };
        let PortableError::ItemRefused(label) = error else {
            panic!("preview={preview} must refuse with ItemRefused, got {error}");
        };
        assert_eq!(label, "analytics.public.customers");
        assert!(
            !error.to_string().contains("Select the latest row"),
            "the refusal names the object, never the claim text: {error}"
        );
    }
    let rows = store.knowledge_for_profile(&identity()).await.unwrap();
    assert_eq!(rows.len(), 0, "nothing written, not even the clean items");
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
/// `count` clean table-description claims, each on its own table and slot.
fn n_table_document(count: usize) -> (SchemaTree, Vec<ContextItem>) {
    let tree = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: (0..count)
                    .map(|index| Table {
                        name: format!("table_{index}"),
                        columns: vec![Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: true,
                        }],
                        primary_key: vec![],
                        foreign_keys: vec![],
                    })
                    .collect(),
            }],
        }],
    };
    let items = (0..count)
        .map(|index| {
            item(
                &format!("table_{index}"),
                &ClaimPayload::table_description(format!("One row per order {index}.")).unwrap(),
            )
        })
        .collect();
    (tree, items)
}
#[tokio::test]
async fn mapping_over_the_batch_cap_refuses_preview_and_apply_alike() {
    let root = std::env::temp_dir().join(format!(
        "saya-portable-cap501-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let (schema, items) = n_table_document(saya_store::MAX_PENDING_BATCH_ITEMS + 1);
    for preview in [true, false] {
        let refused = import_document(&store, &identity(), &items, &schema, preview).await;
        assert!(matches!(
            refused,
            Err(PortableError::TooManyMappedItems(501, 500))
        ));
    }
    let rows = store.knowledge_for_profile(&identity()).await.unwrap();
    assert_eq!(rows.len(), 0, "nothing written");
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
