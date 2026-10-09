use super::*;

/// Spec test 4: two column-scoped slots for different columns coexist.
#[tokio::test]
async fn column_scoped_slots_for_different_columns_coexist() {
    let root = temp_root("t4");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::ColumnRole {
                column: "created_at".into(),
            },
            ClaimPayload::column_role("created_at", ColumnRole::Timestamp, None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::ColumnRole {
                column: "updated_at".into(),
            },
            ClaimPayload::column_role("updated_at", ColumnRole::Timestamp, None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items.len(), 2, "two column-scoped slots coexist");
    let columns: Vec<String> = items
        .iter()
        .filter_map(|item| item.slot.column().map(String::from))
        .collect();
    assert!(columns.contains(&"created_at".to_string()));
    assert!(columns.contains(&"updated_at".to_string()));
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn test_single_valued_slot_db_constraint() {
    let root = temp_root("single-constraint");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "customers");

    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("first grain", None).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();

    // Direct insert attempting to insert a second single-valued slot row for the same object+slot
    let pool = read_pool(&db).await;
    let collision = sqlx::query(
        "INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES ('ki-direct-collision', ?, ?, ?, ?, ?, 'table.grain', 'single', '{}', 'user_explicit', 'active', '{}', 1, 1, 1)",
    )
    .bind(obj.profile().as_str())
    .bind(obj.catalog())
    .bind(obj.schema())
    .bind(obj.object())
    .bind(obj.kind().as_str())
    .execute(&pool)
    .await;

    assert!(
        collision.is_err(),
        "partial unique index knowledge_items_single must reject direct second row for single-valued slot"
    );
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}
