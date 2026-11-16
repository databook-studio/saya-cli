use super::*;

#[test]
fn crash_recovery_marks_prepared_uncertain_and_removes_only_orphans_and_temps() {
    let (state, store) = ck_store("crash");
    let bytes = b"crash survivor\r\n";
    let record = prepare(&store, SESSION, "a.txt", Some(bytes), 1_000).unwrap();
    let dir = ck_dir(&state, SESSION);
    let orphan = dir.join("ck-abababababababababababababababab.before");
    fs::write(&orphan, "orphan bytes").unwrap();
    let temp = dir.join(".x.tmp");
    fs::write(&temp, "staging leftover").unwrap();
    let foreign = dir.join("notes.txt");
    fs::write(&foreign, "keep me").unwrap();
    let garbage = dir.join("ck-cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd.json");
    fs::write(&garbage, "not json").unwrap();

    let listed = store.list_checkpoints(SESSION).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, CheckpointState::Prepared);
    let report = store.recover_checkpoints(SESSION).unwrap();
    assert_eq!(
        report,
        RecoveryReport {
            marked_uncertain: 1,
            orphans_removed: 2,
            skipped: 2
        }
    );
    assert!(!orphan.exists());
    assert!(!temp.exists());
    assert!(foreign.exists(), "foreign files are never touched");
    assert!(garbage.exists(), "unparsable records are never deleted");
    let listed = store.list_checkpoints(SESSION).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, CheckpointState::Uncertain);
    assert_eq!(
        store
            .read_before_image(SESSION, record.id.as_str())
            .unwrap(),
        bytes.to_vec()
    );
    assert_eq!(store.checkpoint_usage(SESSION).unwrap().edits, 1);
    let second = store.recover_checkpoints(SESSION).unwrap();
    assert_eq!(
        second,
        RecoveryReport {
            marked_uncertain: 0,
            orphans_removed: 0,
            skipped: 2
        }
    );
    let _ = fs::remove_dir_all(state);
}
#[cfg(unix)]
#[test]
fn a_failed_prepare_leaves_no_partial_state() {
    use std::os::unix::fs::PermissionsExt;

    struct Restore<'a> {
        path: &'a Path,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.path, fs::Permissions::from_mode(0o700));
        }
    }

    let (state, store) = ck_store("read-only");
    prepare(&store, SESSION, "a.txt", Some(b"kept"), 1_000).unwrap();
    let dir = ck_dir(&state, SESSION);
    let before_count = entry_count(&dir);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
    let _guard = Restore { path: &dir };
    let error = prepare(&store, SESSION, "b.txt", Some(b"new"), 1_000).unwrap_err();
    assert!(matches!(error, CheckpointStoreError::Store(_)));
    assert_eq!(
        entry_count(&dir),
        before_count,
        "the failed prepare left no image, record, or staging temp"
    );
    drop(_guard);
    prepare(&store, SESSION, "b.txt", Some(b"new"), 1_000).unwrap();
    let _ = fs::remove_dir_all(state);
}
#[test]
fn read_before_image_refuses_tampered_bytes() {
    let (state, store) = ck_store("tamper");
    let bytes = b"tamper target bytes";
    let record = prepare(&store, SESSION, "a.txt", Some(bytes), 1_000).unwrap();
    let image = ck_dir(&state, SESSION).join(format!("{}.before", record.id.as_str()));
    let mut stored = fs::read(&image).unwrap();
    stored[0] ^= 0xff;
    fs::write(&image, &stored).unwrap();
    assert_eq!(
        store
            .read_before_image(SESSION, record.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::Corrupt
    );

    let other = prepare(&store, SESSION, "b.txt", Some(bytes), 1_000).unwrap();
    let image = ck_dir(&state, SESSION).join(format!("{}.before", other.id.as_str()));
    let mut stored = fs::read(&image).unwrap();
    stored.pop();
    fs::write(&image, &stored).unwrap();
    assert_eq!(
        store
            .read_before_image(SESSION, other.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::Corrupt
    );
    let _ = fs::remove_dir_all(state);
}
