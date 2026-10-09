use super::*;

// Test 2: forgetting reverts immediately
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forgetting_the_claim_makes_the_block_disappear() {
    use saya_store::KnowledgeItemStore;
    let root = temp_root("forget");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let (obj, _fp) = seed_orders_with_created_at(&store, &identity).await;
    let registry = registry_for("analytics", &identity);

    let (before, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(before.len(), 1);

    // The D-3 store path the contract tools use to forget a knowledge item:
    // dismiss it. Admissibility excludes `Dismissed`, so the item no longer
    // reaches recall and the block reverts immediately.
    let item_id = store
        .knowledge_for_object(&obj)
        .await
        .expect("knowledge items listed")
        .into_iter()
        .find(|i| i.slot == KnowledgeSlot::TableDefaultTime)
        .expect("the confirmed item is stored")
        .id;
    store
        .update_knowledge_item_state(&item_id, KnowledgeState::Dismissed)
        .await
        .expect("item dismissed");

    let (after, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert!(after.is_empty(), "forgetting reverts the block immediately");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn candidate_claim_never_appears_in_block() {
    let root = temp_root("candidate");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");
    let tree = orders_schema(&identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    remember_candidate_default_time_column(&store, &obj, &fp, "created_at").await;

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
    assert!(blocks.is_empty(), "a candidate claim produces no block");
    let _ = fs::remove_dir_all(root);
}
