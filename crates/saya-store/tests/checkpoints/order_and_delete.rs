use super::*;

#[test]
fn list_is_sorted_by_created_then_id() {
    let (state, store) = ck_store("order");
    prepare(&store, SESSION, "b.txt", Some(b"1"), 2_000).unwrap();
    prepare(&store, SESSION, "a.txt", Some(b"2"), 1_000).unwrap();
    prepare(&store, SESSION, "c.txt", Some(b"3"), 2_000).unwrap();
    let listed = store.list_checkpoints(SESSION).unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].path, "a.txt");
    assert!(
        listed
            .windows(2)
            .all(|pair| (pair[0].created_unix_ms, pair[0].id.as_str())
                < (pair[1].created_unix_ms, pair[1].id.as_str()))
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn prepare_writes_the_bytes_before_the_record() {
    let (state, store) = ck_store("write-order");
    let record = prepare(&store, SESSION, "a.txt", Some(b"ordered"), 1_000).unwrap();
    let dir = ck_dir(&state, SESSION);
    let image_mtime = fs::symlink_metadata(dir.join(format!("{}.before", record.id.as_str())))
        .unwrap()
        .modified()
        .unwrap();
    let record_mtime = fs::symlink_metadata(dir.join(format!("{}.json", record.id.as_str())))
        .unwrap()
        .modified()
        .unwrap();
    assert!(
        image_mtime <= record_mtime,
        "the before-image bytes must land before the Prepared record: a crash in \
         between must leave an orphan image, never a record without its image"
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn delete_checkpoint_removes_record_and_image_and_is_idempotent() {
    let (state, store) = ck_store("delete-ck");
    let record = prepare(&store, SESSION, "a.txt", Some(b"v1"), 1_000).unwrap();
    let dir = ck_dir(&state, SESSION);
    assert_eq!(entry_count(&dir), 2);
    store
        .delete_checkpoint(SESSION, record.id.as_str())
        .unwrap();
    assert_eq!(entry_count(&dir), 0);
    store
        .delete_checkpoint(SESSION, record.id.as_str())
        .unwrap();
    assert_eq!(
        store.checkpoint_usage(SESSION).unwrap(),
        CheckpointUsage::default()
    );
    assert_eq!(
        store
            .read_before_image(SESSION, record.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::NotFound
    );
    // Deleting an unknown id is the same idempotent no-op; only a
    // malformed id is refused.
    store
        .delete_checkpoint(SESSION, "ck-00000000000000000000000000000000")
        .unwrap();
    assert_eq!(
        store.delete_checkpoint(SESSION, "garbage").unwrap_err(),
        CheckpointStoreError::NotFound
    );
    let _ = fs::remove_dir_all(state);
}
