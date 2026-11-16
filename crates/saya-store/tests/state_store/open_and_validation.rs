use super::*;

#[tokio::test]
async fn independent_stores_serialize_first_initialization_and_corrupt_files_fail_safely() {
    let root = temp_root("concurrent");
    let db = root.join("state.sqlite3");
    fs::write(&db, []).unwrap();
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let store = SqliteStateStore::new(&db);
        tasks.push(tokio::spawn(async move {
            store
                .upsert_schema(PROFILE, &schema("events"))
                .await
                .unwrap();
            store
                .record_audit(AuditEntry::new(
                    PROFILE,
                    AuditOperation::SchemaRefresh,
                    AuditStatus::Success,
                    1,
                ))
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        SqliteStateStore::new(&db)
            .recent_audit(100)
            .await
            .unwrap()
            .len(),
        12
    );
    let corrupt_root = temp_root("corrupt");
    let corrupt = corrupt_root.join("state.sqlite3");
    fs::write(&corrupt, b"server error /private/tmp password").unwrap();
    let error = SqliteStateStore::new(corrupt)
        .get_schema(PROFILE)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::OpenFailed);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(corrupt_root);
}
#[tokio::test]
async fn adversarial_labels_are_rejected_and_never_written() {
    let root = temp_root("sentinels");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    for value in [
        "password=secret",
        "PRIVATE KEY",
        "SELECT raw_literal",
        "prompt/row/path",
    ] {
        assert!(store.upsert_schema(value, &schema("safe")).await.is_err());
        assert!(
            store
                .record_audit(AuditEntry::new(
                    value,
                    AuditOperation::Query,
                    AuditStatus::Failure,
                    1
                ))
                .await
                .is_err()
        );
    }
    let bytes = fs::read(&db).unwrap_or_default();
    let bytes = String::from_utf8_lossy(&bytes).into_owned();
    for value in [
        "password=secret",
        "PRIVATE KEY",
        "SELECT raw_literal",
        "prompt/row/path",
    ] {
        assert!(!bytes.contains(value));
    }
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn malformed_profile_id_is_rejected_as_invalid() {
    let root = temp_root("bad-profile-id");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let error = store
        .upsert_schema("not-a-profile-id", &schema("events"))
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    let _ = fs::remove_dir_all(root);
}
