use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SchemaStore, SqliteStateStore, StoreError,
};
use saya_types::{Column, Database, Schema, SchemaTree, Table};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const PROFILE: &str = "p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn schema_roundtrip_upsert_invalidate_and_versioned_reopen() {
    let root = temp_root("schema");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    let cached = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached.schema, schema("events"));
    assert_eq!(cached.version, 1);
    store
        .upsert_schema(PROFILE, &schema("events_v2"))
        .await
        .unwrap();
    assert_eq!(
        SqliteStateStore::new(&db)
            .get_schema(PROFILE)
            .await
            .unwrap()
            .unwrap()
            .schema,
        schema("events_v2")
    );
    assert_eq!(store.list_schema_metadata().await.unwrap().len(), 1);
    store.invalidate_schema(PROFILE).await.unwrap();
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn audit_is_typed_bounded_retained_and_decoded_without_payload_fields() {
    let root = temp_root("audit");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    for index in 0..1_005 {
        let mut event = AuditEntry::new(PROFILE, AuditOperation::Query, AuditStatus::Success, 5);
        event.row_count = Some(index);
        event.truncated = Some(false);
        store.record_audit(event).await.unwrap();
    }
    let audit = store.recent_audit(2_000).await.unwrap();
    assert_eq!(audit.len(), 1_000);
    assert!(
        audit
            .iter()
            .all(|row| row.event.operation == AuditOperation::Query
                && row.event.profile_id == PROFILE)
    );
    assert!(store.recent_audit(20_000).await.unwrap().len() <= 1_000);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn concurrent_access_empty_and_corrupt_files_fail_safely() {
    let root = temp_root("concurrent");
    let db = root.join("state.sqlite3");
    fs::write(&db, []).unwrap();
    let store = SqliteStateStore::new(&db);
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let store = store.clone();
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
    assert_eq!(store.recent_audit(100).await.unwrap().len(), 12);
    let corrupt_root = temp_root("corrupt");
    let corrupt = corrupt_root.join("state.sqlite3");
    fs::write(&corrupt, b"server error /private/tmp password").unwrap();
    let error = SqliteStateStore::new(corrupt)
        .get_schema(PROFILE)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "local state store is unavailable");
    assert_eq!(format!("{error:?}"), "Unavailable");
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

#[cfg(unix)]
#[tokio::test]
async fn unix_parent_database_and_sidecars_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("permissions");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for entry in fs::read_dir(&root).unwrap().flatten() {
        assert_eq!(
            entry.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn non_utf8_sidecar_paths_are_lossless() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b's', 0xff, b'a']));
    assert_eq!(
        saya_store::state_sidecar_path(&path, "-wal")
            .as_os_str()
            .as_bytes(),
        b"s\xffa-wal"
    );
}

#[test]
fn store_errors_are_payload_free() {
    assert_eq!(format!("{:?}", StoreError::Unavailable), "Unavailable");
}

fn schema(table: &str) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "main".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: table.into(),
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "INTEGER".into(),
                        nullable: false,
                    }],
                }],
            }],
        }],
    }
}
fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-state-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
