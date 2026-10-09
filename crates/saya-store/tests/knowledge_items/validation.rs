use super::*;

/// Spec test 7 (rewritten for Chunk 5): the legacy `contract_*` tables are
/// *gone* after the step-6 migration, and `knowledge_items` is the sole store.
/// The original test asserted the legacy claim path coexisted with the new
/// knowledge table; Chunk 5 dropped that path and those tables, so the
/// equivalent guarantee now is that a migrated database has no `contract_*`
/// tables and a knowledge write still round-trips on its own — the knowledge
/// path stands alone, not alongside a legacy one.
#[tokio::test]
async fn legacy_contract_tables_are_dropped_and_knowledge_stands_alone() {
    let root = temp_root("t7");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    // The knowledge write is the only path now; it round-trips on its own.
    store
        .put_knowledge_item(request_knowledge(
            &obj,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("one row per order", None).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(store.knowledge_for_object(&obj).await.unwrap().len(), 1);

    // Step 6 dropped every `contract_*` table. A migrated database has none,
    // so querying one is an error — the legacy store is not merely unused,
    // it is gone.
    let pool = read_pool(&db).await;
    let legacy: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'contract_%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    pool.close().await;
    assert_eq!(legacy, 0, "step 6 must drop every contract_* table");
    let _ = fs::remove_dir_all(root);
}
/// A mismatched payload under a slot is a typed `CardinalityMismatch`, storing
/// nothing. Not one of the seven spec tests, but the typed-refusal surface is
/// part of the deliverable.
#[tokio::test]
async fn mismatched_payload_under_a_slot_is_refused() {
    let root = temp_root("mismatch");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let error = store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_alias("not a grain").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap_err();
    assert_eq!(error, KnowledgeStoreError::CardinalityMismatch);
    assert!(
        store.knowledge_for_object(&obj).await.unwrap().is_empty(),
        "a refused write must store nothing"
    );
    // Confirm the typed error stays payload-free.
    let rendered = error.to_string();
    assert!(!rendered.contains("not a grain"));
    let _ = fs::remove_dir_all(root);
}
/// A value that structurally resembles a secret is refused, not stored — the
/// same discipline as the claim payload.
#[tokio::test]
async fn secret_shaped_value_is_refused_not_stored() {
    let root = temp_root("secret");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let error = store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            // A description whose text contains a credential header shape.
            ClaimPayload::table_grain("x-api-key: SUPERSECRETVALUE", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await;
    assert!(error.is_err(), "a secret-shaped value must be refused");
    let err = error.unwrap_err();
    assert_eq!(err, KnowledgeStoreError::Store(StoreError::Invalid));
    assert!(
        store.knowledge_for_object(&obj).await.unwrap().is_empty(),
        "a refused value must not be stored"
    );
    let _ = fs::remove_dir_all(root);
}
/// A `relationship` payload matches no slot, so it is refused as a
/// cardinality mismatch — the value vocabulary has no slot to file it under.
#[tokio::test]
async fn relationship_payload_matches_no_slot() {
    let root = temp_root("rel");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let target = object(&profile('a'), "users");
    let rel = ClaimPayload::relationship(
        target,
        vec!["user_id".into()],
        vec!["id".into()],
        saya_types::Cardinality::ManyToOne,
    )
    .unwrap();
    let error = store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            rel,
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap_err();
    assert_eq!(error, KnowledgeStoreError::CardinalityMismatch);
    let _ = fs::remove_dir_all(root);
}
