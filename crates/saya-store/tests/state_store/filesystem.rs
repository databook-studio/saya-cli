use super::*;

#[cfg(unix)]
#[tokio::test]
async fn unix_parent_database_and_sidecars_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("permissions");
    let db = root.join("created").join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    assert_eq!(
        fs::metadata(root.join("created"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for entry in fs::read_dir(root.join("created")).unwrap().flatten() {
        assert_eq!(
            entry.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let _ = fs::remove_dir_all(root);
}
#[cfg(unix)]
#[tokio::test]
async fn unix_existing_state_parent_preserves_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("shared-parent");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o755
    );
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
