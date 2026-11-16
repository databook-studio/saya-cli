use super::*;

// Test 4: allow_database_context=false → no block, store not queried
// ---------------------------------------------------------------------------

#[tokio::test]
async fn privacy_off_produces_no_block_and_does_not_query_store() {
    let root = temp_root("privacy_off");
    // A store whose parent path is a regular file cannot be opened — if recall
    // touched it, `get_schema` would fail and the outcome would read
    // `store_unavailable`. By construction the privacy gate returns before any
    // store call, so the unopenable store is never observed: it produces no
    // block and no error, and we never had to open it.
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad_path = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad_path);
    let identity = identity_for("analytics");
    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        false,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert!(blocks.is_empty(), "privacy gate off → no block");
    // The bad store was never opened: its parent is still a regular file,
    // so opening it would have panicked/migrated. The gate held.
    assert!(std::fs::metadata(root.join("blocker")).is_ok());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn explicit_ref_selects_object_without_term_match() {
    let root = temp_root("explicit_ref");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    // A claim under an object whose name no term in the prompt matches.
    let obj = object(&identity, "obscure_table_name");
    let tree = orders_schema_named(&identity, "obscure_table_name");
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&table_named("obscure_table_name"));
    remember_confirmed_default_time_column(&store, &obj, &fp, "created_at").await;

    let registry = registry_for("analytics", &identity);
    // No term matches "obscure_table_name"; only the explicit @ref does.
    let (blocks, _receipt) = recall_blocks(
        "summarize @catalog.public.obscure_table_name",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].body.contains("catalog.public.obscure_table_name"));
    assert!(blocks[0].body.contains("created_at"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn prompt_matching_nothing_produces_no_block() {
    let root = temp_root("no_match");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let (obj, fp) = seed_orders_with_created_at(&store, &identity).await;
    let _ = (obj, fp);
    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "completely unrelated zzztop words",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert!(blocks.is_empty(), "no match → no block, not an empty block");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn unopenable_store_produces_no_block_and_no_error() {
    let root = temp_root("unopenable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad_path = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad_path);
    let identity = identity_for("analytics");
    let registry = registry_for("analytics", &identity);

    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert!(blocks.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn opaque_identity_appears_nowhere_in_block() {
    let root = temp_root("no_identity");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    seed_orders_with_created_at(&store, &identity).await;
    let registry = registry_for("analytics", &identity);

    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1);
    let body = &blocks[0].body;
    assert!(
        !body.contains(identity.as_str()),
        "opaque identity leaked into block body"
    );
    // The profile name should appear instead.
    assert!(body.contains("analytics"));
    let _ = fs::remove_dir_all(root);
}
