use super::*;

/// A version ahead of the highest step the migration knows about fails closed.
/// Step 13 makes `user_version = 13` supported, so the future-version sentinel is
/// now 14 — anything the running build cannot migrate *to* must be refused, not
/// silently rewritten under.
#[tokio::test]
async fn unknown_future_version_fails_closed() {
    let root = temp_root("future");
    let db = root.join("state.sqlite3");
    let pool = create_pool(&db).await;
    sqlx::query("PRAGMA user_version = 14")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let before = fs::read(&db).unwrap();
    let store = SqliteStateStore::new(&db);
    let error = store.list_schema_metadata().await.unwrap_err();
    assert_eq!(error, StoreError::VersionUnsupported);
    let after = fs::read(&db).unwrap();
    assert_eq!(before, after);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn wal_is_on_after_upgrade() {
    let root = temp_root("wal");
    let db = root.join("state.sqlite3");
    build_version_one_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(journal_mode(&db).await, "wal");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn permissions_survive_after_upgrade() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("perms");
    let db = root.join("state.sqlite3");
    build_version_one_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(
        fs::metadata(&db).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let _ = fs::remove_dir_all(root);
}
