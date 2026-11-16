use super::*;

/// Spec test 5: a read for profile A never returns profile B's rows.
#[tokio::test]
async fn read_for_one_profile_never_returns_anothers_rows() {
    let root = temp_root("t5");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj_a = object(&profile('a'), "orders");
    let obj_b = object(&profile('b'), "orders");
    store
        .put_knowledge_item(request(
            &obj_a,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("profile a grain", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj_b,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("profile b grain", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();

    let a_items = store.knowledge_for_profile(&profile('a')).await.unwrap();
    assert_eq!(a_items.len(), 1);
    assert_eq!(a_items[0].object.profile(), &profile('a'));
    let b_items = store.knowledge_for_profile(&profile('b')).await.unwrap();
    assert_eq!(b_items.len(), 1);
    assert_eq!(b_items[0].object.profile(), &profile('b'));
    // The same qualified name under profile A never leaks into profile B.
    assert!(
        a_items
            .iter()
            .all(|item| item.object.profile() == &profile('a'))
    );
    assert!(
        b_items
            .iter()
            .all(|item| item.object.profile() == &profile('b'))
    );
    let _ = fs::remove_dir_all(root);
}
/// Spec test 6: one query returns everything known for a profile.
#[tokio::test]
async fn one_query_returns_everything_known_for_a_profile() {
    let root = temp_root("t6");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj_orders = object(&profile('a'), "orders");
    let obj_events = object(&profile('a'), "events");
    // Several slots across two objects — a description (multi), a grain
    // (single), and two column roles.
    store
        .put_knowledge_item(request(
            &obj_orders,
            KnowledgeSlot::TableDescription,
            ClaimPayload::table_description("the orders table").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj_orders,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("one row per order", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj_events,
            KnowledgeSlot::ColumnRole {
                column: "occurred_at".into(),
            },
            ClaimPayload::column_role("occurred_at", ColumnRole::Timestamp, None).unwrap(),
            ClaimOrigin::TeamFile,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    let items = store.knowledge_for_profile(&profile('a')).await.unwrap();
    // Three rows across two objects, from one query.
    assert_eq!(items.len(), 3);
    let objects: Vec<&str> = items.iter().map(|i| i.object.object()).collect();
    assert!(objects.contains(&"orders"));
    assert!(objects.contains(&"events"));
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn test_objects_for_profile_returns_distinct_profile_objects() {
    let root = temp_root("objects-distinct");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let prof = profile('a');
    let orders = object(&prof, "orders");
    let line_items = object(&prof, "line_items");

    // Multiple items for orders
    store
        .put_knowledge_item(request(
            &orders,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("one row per order", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &orders,
            KnowledgeSlot::TableDescription,
            ClaimPayload::table_description("orders table").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();

    // One item for line_items
    store
        .put_knowledge_item(request(
            &line_items,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("one row per line item", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();

    let objects = store.objects_for_profile(&prof).await.unwrap();
    assert_eq!(
        objects.len(),
        2,
        "must return deduplicated database objects"
    );
    let names: Vec<&str> = objects.iter().map(|o| o.object()).collect();
    assert_eq!(names, vec!["line_items", "orders"]);
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn test_knowledge_items_profile_scoping() {
    let root = temp_root("profile-scoping");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let prof_a = profile('a');
    let prof_b = profile('b');
    let orders_a = object(&prof_a, "orders");
    let orders_b = object(&prof_b, "orders");

    store
        .put_knowledge_item(request(
            &orders_a,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("profile a grain", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &orders_b,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("profile b grain", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();

    let objs_a = store.objects_for_profile(&prof_a).await.unwrap();
    assert_eq!(objs_a.len(), 1);
    assert_eq!(objs_a[0].profile(), &prof_a);

    let objs_b = store.objects_for_profile(&prof_b).await.unwrap();
    assert_eq!(objs_b.len(), 1);
    assert_eq!(objs_b[0].profile(), &prof_b);

    let items_a = store.knowledge_for_profile(&prof_a).await.unwrap();
    assert_eq!(items_a.len(), 1);
    assert_eq!(items_a[0].object.profile(), &prof_a);

    let items_b = store.knowledge_for_profile(&prof_b).await.unwrap();
    assert_eq!(items_b.len(), 1);
    assert_eq!(items_b[0].object.profile(), &prof_b);
    let _ = fs::remove_dir_all(root);
}
