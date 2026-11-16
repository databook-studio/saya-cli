#[tokio::test]
async fn mapping_exactly_the_batch_cap_applies() {
    let root = std::env::temp_dir().join(format!(
        "saya-portable-cap500-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let (schema, items) = n_table_document(saya_store::MAX_PENDING_BATCH_ITEMS);
    let preview = import_document(&store, &identity(), &items, &schema, true)
        .await
        .unwrap();
    assert_eq!(preview.inserted.len(), 500);
    let applied = import_document(&store, &identity(), &items, &schema, false)
        .await
        .unwrap();
    assert_eq!(applied.inserted.len(), 500);
    let rows = store.knowledge_for_profile(&identity()).await.unwrap();
    assert_eq!(rows.len(), 500);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

/// A resolved target keeps the tree's spelling for the object name: the file
/// may spell `ORDERS` where the schema cache names `orders`, and the ref must
/// carry the tree's name so the slot id is stable across re-imports.
#[test]
fn resolve_ref_keeps_the_tree_spelling_for_the_object_name() {
    let schema = tree(&[("analytics.public", "orders", &[("id", "bigint")])]);
    let object = PortableObject {
        catalog: Some("analytics".into()),
        schema: Some("public".into()),
        name: "ORDERS".into(),
        kind: DatabaseObjectKind::Table,
    };
    let r = super::resolve::resolve_ref(&schema, &identity(), &object)
        .expect("the upper-case target resolves against the tree");
    assert_eq!(r.object(), "orders", "the tree's spelling, not the file's");
    assert_eq!(r.qualified_name(), "analytics.public.orders");
}
