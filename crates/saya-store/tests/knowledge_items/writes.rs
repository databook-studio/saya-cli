use super::*;

/// Spec test 1: insert and read back one item; every field round-trips,
/// including the binding and its version.
#[tokio::test]
async fn insert_and_read_back_round_trips_every_field() {
    let root = temp_root("t1");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let (fingerprint, schema_binding_json) = binding(7);
    let value = ClaimPayload::table_grain("one row per shipped order", None).unwrap();
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: obj.clone(),
            slot: KnowledgeSlot::TableGrain,
            value: value.clone(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: schema_binding_json.clone(),
            fingerprint,
        })
        .await
        .unwrap();

    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.object, obj);
    assert_eq!(item.slot, KnowledgeSlot::TableGrain);
    assert!(item.cardinality_single, "table.grain is single-valued");
    assert_eq!(item.value, value);
    assert_eq!(item.source, ClaimOrigin::UserExplicit);
    assert_eq!(item.state, KnowledgeState::Active);
    // The binding and its version round-trip verbatim — not re-derived.
    assert_eq!(item.schema_binding_json, schema_binding_json);
    assert_eq!(item.fingerprint_version, 7);
    assert!(item.created_unix_ms > 0);
    assert_eq!(item.updated_unix_ms, item.created_unix_ms);
    assert!(item.id.starts_with("ki-"));
    let _ = fs::remove_dir_all(root);
}
/// Spec test 2: writing a second value to a single-valued slot replaces — one
/// row after, not two. Asserted at the database level, not just through the
/// API. The partial unique index is the contract; prove it directly too.
#[tokio::test]
async fn second_value_to_single_slot_replaces_at_the_database() {
    let root = temp_root("t2");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");

    let first = ClaimPayload::table_grain("one row per order", None).unwrap();
    let second = ClaimPayload::table_grain("one row per shipment", None).unwrap();
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            first,
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            second.clone(),
            ClaimOrigin::AssistantInferred,
            KnowledgeState::Pending,
            2,
        ))
        .await
        .unwrap();

    // API read: one item, holding the second value.
    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items.len(), 1, "replace must not grow a single-valued slot");
    assert_eq!(items[0].value, second);
    assert_eq!(items[0].source, ClaimOrigin::AssistantInferred);
    assert_eq!(items[0].state, KnowledgeState::Pending);
    assert_eq!(items[0].fingerprint_version, 2);

    // Database level: exactly one row, and it carries the second value.
    let pool = read_pool(&db).await;
    let (count, value_json, source): (i64, String, String) = sqlx::query_as(
        "SELECT COUNT(*), value_json, source FROM knowledge_items WHERE profile_id=? AND catalog=? AND schema=? AND object=? AND object_kind=? AND slot='table.grain'",
    )
    .bind(obj.profile().as_str())
    .bind(obj.catalog())
    .bind(obj.schema())
    .bind(obj.object())
    .bind(obj.kind().as_str())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let restored: ClaimPayload = serde_json::from_str(&value_json).unwrap();
    assert_eq!(restored, second);
    assert_eq!(source, "assistant_inferred");
    pool.close().await;

    // The unique index itself — independent of the id scheme — refuses two
    // single-valued rows for the same object+slot. Two rows with distinct ids
    // and distinct values must collide on the partial index.
    let pool = read_pool(&db).await;
    sqlx::query("INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES ('ki-direct-a', ?, ?, ?, ?, ?, 'column:created_at.role', 'single', '{}', 'user_explicit', 'active', '{}', 1, 1, 1)")
        .bind(obj.profile().as_str())
        .bind(obj.catalog())
        .bind(obj.schema())
        .bind(obj.object())
        .bind(obj.kind().as_str())
        .execute(&pool)
        .await
        .unwrap();
    let collision = sqlx::query("INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES ('ki-direct-b', ?, ?, ?, ?, ?, 'column:created_at.role', 'single', '{}', 'user_explicit', 'active', '{}', 1, 1, 1)")
        .bind(obj.profile().as_str())
        .bind(obj.catalog())
        .bind(obj.schema())
        .bind(obj.object())
        .bind(obj.kind().as_str())
        .execute(&pool)
        .await;
    assert!(
        collision.is_err(),
        "knowledge_items is missing its single-valued UNIQUE index"
    );
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}
/// Spec test 3: a multi-valued slot accepts more than one value, up to its
/// bound, and refuses past it with a typed error.
#[tokio::test]
async fn multi_valued_slot_accepts_up_to_its_bound() {
    let root = temp_root("t3");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let slot = KnowledgeSlot::TableAlias;
    for i in 0..saya_types::MAX_MULTI_SLOT_VALUES {
        let alias = ClaimPayload::table_alias(format!("alias{i}")).unwrap();
        store
            .put_knowledge_item(request(
                &obj,
                slot.clone(),
                alias,
                ClaimOrigin::UserExplicit,
                KnowledgeState::Active,
                1,
            ))
            .await
            .unwrap();
    }
    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(
        items.len(),
        saya_types::MAX_MULTI_SLOT_VALUES,
        "multi-valued slot accepts up to its bound"
    );
    // One past the bound is a typed refusal, and it stores nothing.
    let too_many = ClaimPayload::table_alias("one too many").unwrap();
    let error = store
        .put_knowledge_item(request(
            &obj,
            slot,
            too_many,
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap_err();
    assert_eq!(error, KnowledgeStoreError::BoundExceeded);
    assert_eq!(
        store.knowledge_for_object(&obj).await.unwrap().len(),
        saya_types::MAX_MULTI_SLOT_VALUES,
        "a refused append must store nothing"
    );
    // Re-filing an existing value is an idempotent update, not a duplicate and
    // not a bound refusal.
    let again = ClaimPayload::table_alias("alias0").unwrap();
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableAlias,
            again,
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    assert_eq!(
        store.knowledge_for_object(&obj).await.unwrap().len(),
        saya_types::MAX_MULTI_SLOT_VALUES
    );
    let _ = fs::remove_dir_all(root);
}
