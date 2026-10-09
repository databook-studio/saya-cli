use super::*;

// Test (no profiles / empty prompt): recall does not run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_profiles_produces_no_block() {
    let root = temp_root("no_profiles");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    seed_orders_with_created_at(&store, &identity).await;
    // A registry with no identity-bearing entry.
    let mut registry = ConnectionRegistry::new("analytics");
    registry.insert(
        "analytics",
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
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
async fn empty_prompt_produces_no_block() {
    let root = temp_root("empty_prompt");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    seed_orders_with_created_at(&store, &identity).await;
    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "   ",
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
async fn recall_truncation_flags_the_block() {
    let root = temp_root("truncated");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    // Many matching objects beyond the default max_objects=5 → truncation.
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
    let (blocks, _receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].truncated, "recall truncation must flag the block");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn no_state_db_produces_no_block() {
    let root = temp_root("no_store");
    let identity = identity_for("analytics");
    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        None,
    )
    .await;
    assert!(blocks.is_empty());
    let _ = root; // nothing was written
}

/// A direct check that recall's outcome text is exactly what render_body
/// produces, and that the body alone — never a system field — carries it.
/// `system_prompt` for this layer is built in `runtime.rs` from the registry's
/// `describe_context`, which names connections and dialects only. This test
/// asserts the structural guarantee: claim text exists only in a block body.
#[tokio::test]
async fn claim_text_lives_only_in_block_body_not_describe_context() {
    let root = temp_root("system_purity");
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

    // The system-prompt source for a single-connection registry is `None`.
    let system_prompt = registry.describe_context();
    assert!(
        system_prompt.is_none(),
        "single-connection describe_context is None — no claim text can ride there"
    );
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].body.contains("created_at"));
    let _ = fs::remove_dir_all(root);
}
