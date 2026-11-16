use super::*;

#[tokio::test]
async fn test_get_knowledge_item_by_id_returns_exact_item() {
    let root = temp_root("get-by-id");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let (fingerprint, schema_binding_json) = binding(3);
    let value = ClaimPayload::table_grain("one row per order", None).unwrap();
    let req = KnowledgeItemRequest {
        object: obj.clone(),
        slot: KnowledgeSlot::TableGrain,
        value: value.clone(),
        source: ClaimOrigin::UserExplicit,
        state: KnowledgeState::Active,
        schema_binding_json: schema_binding_json.clone(),
        fingerprint,
    };
    store.put_knowledge_item(req).await.unwrap();

    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items.len(), 1);
    let id = &items[0].id;

    let fetched = store.get_knowledge_item(id).await.unwrap();
    assert!(fetched.is_some());
    let item = fetched.unwrap();
    assert_eq!(item.id, *id);
    assert_eq!(item.object, obj);
    assert_eq!(item.slot, KnowledgeSlot::TableGrain);
    assert_eq!(item.value, value);
    assert_eq!(item.source, ClaimOrigin::UserExplicit);
    assert_eq!(item.state, KnowledgeState::Active);
    assert_eq!(item.schema_binding_json, schema_binding_json);
    assert_eq!(item.fingerprint_version, 3);
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn test_get_knowledge_item_returns_none_for_missing_id() {
    let root = temp_root("get-none");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let fetched = store.get_knowledge_item("ki-nonexistent-id").await.unwrap();
    assert_eq!(fetched, None);
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn test_update_knowledge_item_state_transitions() {
    let root = temp_root("update-state");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let req = request(
        &obj,
        KnowledgeSlot::TableGrain,
        ClaimPayload::table_grain("grain", None).unwrap(),
        ClaimOrigin::AssistantInferred,
        KnowledgeState::Pending,
        1,
    );
    store.put_knowledge_item(req).await.unwrap();
    let items = store.knowledge_for_object(&obj).await.unwrap();
    let id = &items[0].id;
    assert_eq!(items[0].state, KnowledgeState::Pending);

    // Transition to Active
    store
        .update_knowledge_item_state(id, KnowledgeState::Active)
        .await
        .unwrap();
    let item = store.get_knowledge_item(id).await.unwrap().unwrap();
    assert_eq!(item.state, KnowledgeState::Active);
    assert!(item.updated_unix_ms >= item.created_unix_ms);

    // Transition to Dismissed
    store
        .update_knowledge_item_state(id, KnowledgeState::Dismissed)
        .await
        .unwrap();
    let item = store.get_knowledge_item(id).await.unwrap().unwrap();
    assert_eq!(item.state, KnowledgeState::Dismissed);
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn test_revalidate_knowledge_item_updates_binding_and_activates() {
    let root = temp_root("revalidate");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let req = request(
        &obj,
        KnowledgeSlot::TableGrain,
        ClaimPayload::table_grain("grain", None).unwrap(),
        ClaimOrigin::AssistantInferred,
        KnowledgeState::Pending,
        1,
    );
    store.put_knowledge_item(req).await.unwrap();
    let items = store.knowledge_for_object(&obj).await.unwrap();
    let id = &items[0].id;
    assert_eq!(items[0].state, KnowledgeState::Pending);
    assert_eq!(items[0].fingerprint_version, 1);

    let (new_fp, new_binding) = binding(5);
    store
        .revalidate_knowledge_item(id, new_fp, new_binding.clone())
        .await
        .unwrap();

    let item = store.get_knowledge_item(id).await.unwrap().unwrap();
    assert_eq!(item.state, KnowledgeState::Active);
    assert_eq!(item.fingerprint_version, 5);
    assert_eq!(item.schema_binding_json, new_binding);
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn schema_binding_size_is_capped_on_insert_and_revalidate() {
    let root = temp_root("binding-cap");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let oversized = "x".repeat(MAX_SCHEMA_BINDING_BYTES + 1);
    let mut req = request(
        &obj,
        KnowledgeSlot::TableGrain,
        ClaimPayload::table_grain("grain", None).unwrap(),
        ClaimOrigin::UserExplicit,
        KnowledgeState::Pending,
        1,
    );
    req.schema_binding_json = oversized.clone();
    assert_eq!(
        store.put_knowledge_item(req).await,
        Err(KnowledgeStoreError::Store(StoreError::LimitExceeded))
    );
    assert!(store.knowledge_for_object(&obj).await.unwrap().is_empty());

    let req = request(
        &obj,
        KnowledgeSlot::TableGrain,
        ClaimPayload::table_grain("grain", None).unwrap(),
        ClaimOrigin::UserExplicit,
        KnowledgeState::Pending,
        1,
    );
    store.put_knowledge_item(req.clone()).await.unwrap();
    let id = store.knowledge_for_object(&obj).await.unwrap().remove(0).id;
    assert_eq!(
        store
            .revalidate_knowledge_item(&id, req.fingerprint, oversized,)
            .await,
        Err(KnowledgeStoreError::Store(StoreError::LimitExceeded))
    );
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn test_delete_knowledge_item_removes_row() {
    let root = temp_root("delete");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let req = request(
        &obj,
        KnowledgeSlot::TableGrain,
        ClaimPayload::table_grain("grain", None).unwrap(),
        ClaimOrigin::UserExplicit,
        KnowledgeState::Active,
        1,
    );
    store.put_knowledge_item(req).await.unwrap();
    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items.len(), 1);
    let id = &items[0].id;

    store.delete_knowledge_item(id).await.unwrap();
    let fetched = store.get_knowledge_item(id).await.unwrap();
    assert_eq!(fetched, None);
    assert!(store.knowledge_for_object(&obj).await.unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}
