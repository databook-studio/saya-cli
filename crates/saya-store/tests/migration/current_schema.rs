use super::*;

#[tokio::test]
async fn fresh_database_reaches_latest_version() {
    let root = temp_root("fresh");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 11);
    // Step 6 dropped the legacy `contract_*` tables; a fresh database has none.
    assert_eq!(contract_tables(&db).await.len(), 0);
    assert!(
        table_exists(&db, "user_preferences").await,
        "missing user_preferences"
    );
    // Step 5 adds the knowledge_items table; a fresh database reaches it.
    assert!(
        table_exists(&db, "knowledge_items").await,
        "missing knowledge_items"
    );
    assert!(
        column_exists(&db, "knowledge_items", "cleanup_state").await,
        "missing cleanup state"
    );
    // Step 7 adds the run-spine tables; a fresh database reaches them.
    assert!(table_exists(&db, "runs").await, "missing runs");
    assert!(table_exists(&db, "run_steps").await, "missing run_steps");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn migration_is_idempotent() {
    let root = temp_root("idempotent");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 11);
    let reopened = SqliteStateStore::new(&db);
    reopened.list_schema_metadata().await.unwrap();
    reopened.close().await;
    assert_eq!(user_version(&db).await, 11);
    let _ = fs::remove_dir_all(root);
}
