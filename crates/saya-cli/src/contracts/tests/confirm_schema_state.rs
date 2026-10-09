#[tokio::test]
async fn confirm_candidate_against_an_empty_cached_schema_succeeds_not_object_gone() {
    let root = temp_root("confirm_empty_schema_candidate");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // `store_at` cached `SchemaTree::default()` (empty `databases`) — leave it.
    // An empty tree is "no schema", not "every object is gone".
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("the orders table").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;

    let confirmed = confirm(&store, &id).await.unwrap();
    assert_eq!(confirmed.status, ClaimStatus::Confirmed);
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

// --- The same empty cache against an `Active` fact (a re-verification) refuses
// --- `SchemaUnavailable`, not `ObjectGone`: there is nothing to verify
// --- against, and an empty cache is not proof the table is gone. The asymmetry
// --- the D-3 translation preserves — a candidate needs no schema; a
// --- re-verification does — holds for an empty cache just as it does for a
// --- missing one.
#[tokio::test]
async fn confirm_active_against_an_empty_cached_schema_is_schema_unavailable_not_object_gone() {
    let root = temp_root("confirm_empty_schema_active");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    // Cache an *empty* tree (not `invalidate_schema` → `Missing`): an empty
    // `Available` tree is the case that used to read `ObjectGone`.
    store
        .upsert_schema(p.as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let err = confirm(&store, &id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::SchemaUnavailable,
        "an empty cache is no schema to verify against, not proof the table is gone"
    );
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

// --- A *populated* schema that lacks the object's table is genuine evidence
// --- the table is gone: `ObjectGone` for either state. This is the case that
// --- stays `ObjectGone` after the empty-cache fix — the cache actually says
// --- something, and what it says is "no such table".
#[tokio::test]
async fn confirm_against_a_populated_schema_lacking_the_table_is_object_gone() {
    let root = temp_root("confirm_populated_no_table");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    // A schema with a *different* table present — populated, but `orders` is
    // absent. The cache genuinely says `orders` is gone.
    let populated = schema_tree_for(&[("shipments", table(&[("id", "bigint", false)]))]);
    store.upsert_schema(p.as_str(), &populated).await.unwrap();

    let err = confirm(&store, &id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::ObjectGone,
        "a populated schema that lacks the table is evidence the table is gone"
    );
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

// --- Confirming a fact whose dependency is gone refuses with a message naming
// --- the obstacle and the repair, rather than reporting a conflict with a
// --- claim that does not exist. Found by running the binary: both refusals
// --- rendered identically under the legacy store; the D-4 refusal keeps the
// --- same message.
#[tokio::test]
async fn confirming_a_stale_claim_names_the_missing_column_and_the_repair() {
    let root = temp_root("confirm-colgone");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    let drifted = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    store.upsert_schema(p.as_str(), &drifted).await.unwrap();

    let error = confirm(&store, &id).await.unwrap_err();
    assert_eq!(error, ContractOpError::ColumnGone);
    let rendered = error.to_string();
    assert!(
        rendered.contains("column"),
        "names the obstacle: {rendered}"
    );
    assert!(
        rendered.contains("edit") && rendered.contains("forget"),
        "names both repairs: {rendered}"
    );
    assert!(
        !rendered.contains("conflict"),
        "must not claim a conflict with another claim: {rendered}"
    );
    let _ = fs::remove_dir_all(root);
}
