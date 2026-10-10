use super::*;

/// Preferences survive `close()` and reopen; a forgotten row is really gone
/// from the file, not merely from the list.
#[tokio::test]
async fn preferences_persist_across_close_and_forget_removes_the_row() {
    let root = temp_root("persist");
    let db = root.join("state.sqlite3");
    let wt = worktree_a();

    let store = SqliteStateStore::new(&db);
    let stored = store
        .add_project_preference(&wt, "persistent guidance", 1_000)
        .await
        .unwrap();
    store.close().await;

    let reopened = SqliteStateStore::new(&db);
    let listed = reopened.list_project_preferences(&wt).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], stored);
    assert_eq!(
        reopened.forget_project_preference(&wt, &stored.id).await,
        Ok(true)
    );
    reopened.close().await;
    assert_eq!(table_count(&db, "project_preferences").await, 0);

    let _ = fs::remove_dir_all(root);
}

/// The typed `PreferenceStore` is untouched: its own table holds only its own
/// rows, and a project preference's text never leaks through it.
#[tokio::test]
async fn the_typed_preference_store_is_unaffected() {
    let root = temp_root("separation");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let scope = PreferenceScope::Global;
    let wt = worktree_a();

    store
        .set_preference(&scope, PreferenceValue::output_style(OutputStyle::Compact))
        .await
        .unwrap();
    store
        .add_project_preference(&wt, "project-only guidance", 1_000)
        .await
        .unwrap();

    let typed = store
        .get_preference(&scope, "output_style")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(typed, PreferenceValue::output_style(OutputStyle::Compact));
    let typed_list = store.list_preferences(&scope).await.unwrap();
    assert_eq!(typed_list.len(), 1);
    assert_eq!(typed_list[0].0, "output_style");
    assert_eq!(table_count(&db, "user_preferences").await, 1);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// A fresh database reaches `user_version = 11` with the project-preferences
/// table in place.
#[tokio::test]
async fn fresh_database_reaches_version_eleven_with_the_project_table() {
    let root = temp_root("fresh-v10");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    assert!(
        store
            .list_project_preferences(&worktree_a())
            .await
            .unwrap()
            .is_empty()
    );
    store.close().await;
    assert_eq!(user_version(&db).await, 11);
    assert!(
        table_exists(&db, "project_preferences").await,
        "missing project_preferences"
    );
    let _ = fs::remove_dir_all(root);
}

/// A database rewound to `user_version = 8` with the project table dropped is
/// upgraded again: the table is recreated at version 11 and other data — here
/// a typed preference — survives the ride.
#[tokio::test]
async fn rewind_to_version_eight_recreates_the_project_table() {
    let root = temp_root("rewind-v8");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .set_preference(
            &PreferenceScope::Global,
            PreferenceValue::output_style(OutputStyle::Compact),
        )
        .await
        .unwrap();
    store.close().await;

    let pool = read_pool(&db).await;
    sqlx::query("DROP TABLE IF EXISTS project_preferences")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_events")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_children")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_jobs")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 8")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(user_version(&db).await, 8);

    let reopened = SqliteStateStore::new(&db);
    assert!(
        reopened
            .list_project_preferences(&worktree_a())
            .await
            .unwrap()
            .is_empty()
    );
    reopened.close().await;
    assert_eq!(user_version(&db).await, 11);
    assert!(table_exists(&db, "project_preferences").await);
    let survivor = SqliteStateStore::new(&db);
    let typed = survivor
        .get_preference(&PreferenceScope::Global, "output_style")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(typed, PreferenceValue::output_style(OutputStyle::Compact));
    survivor.close().await;

    let _ = fs::remove_dir_all(root);
}

/// Every returned record round-trips through the strict reader: version,
/// id shape, worktree shape, text bounds, and timestamps all validate.
#[tokio::test]
async fn every_listed_record_validates_as_json() {
    let root = temp_root("json");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();

    store
        .add_project_preference(&wt, "line one\nline two", 1_000)
        .await
        .unwrap();
    let listed = store.list_project_preferences(&wt).await.unwrap();
    assert_eq!(listed.len(), 1);
    let bytes = serde_json::to_vec(&listed[0]).unwrap();
    let decoded = ProjectPreference::from_json_bytes(&bytes).unwrap();
    assert_eq!(decoded, listed[0]);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}
