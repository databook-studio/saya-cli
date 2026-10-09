use super::*;

#[cfg(unix)]
#[test]
fn a_partial_cleanup_reports_pending_and_a_retry_converges() {
    use std::os::unix::fs::PermissionsExt;

    let (state, store) = deletion_store("pending-retry");
    plant_session(&store, "a", "hello");
    let sessions = state.join("sessions");
    let stubborn = sessions.join("a/stubborn");
    std::fs::create_dir_all(&stubborn).unwrap();
    std::fs::write(stubborn.join("x.json"), "stuck").unwrap();
    // Restores write access even on a failed assert, so the temp tree
    // stays removable.
    struct RestoreMode(std::path::PathBuf);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        }
    }
    let _guard = RestoreMode(stubborn.clone());
    std::fs::set_permissions(&stubborn, std::fs::Permissions::from_mode(0o500)).unwrap();

    store.begin_delete("a").unwrap();
    let status = store.finish_delete("a").unwrap();

    assert!(
        matches!(status, DeleteStatus::Pending { remaining } if remaining >= 1),
        "a blocked child reports pending: {status:?}"
    );
    assert!(
        state.join("lifecycle/session-a.tombstone").is_file(),
        "the tombstone is kept for retry"
    );
    assert!(store.is_deleted("a").unwrap());
    assert!(block_on(store.load("a")).unwrap().is_none(), "still hidden");

    std::fs::set_permissions(&stubborn, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);
    assert!(!sessions.join("a/stubborn/x.json").exists());
    assert!(!sessions.join("a").exists());
    assert!(!state.join("lifecycle/session-a.tombstone").exists());
    let _ = std::fs::remove_dir_all(state);
}
#[test]
fn repeated_begin_and_finish_are_idempotent() {
    let (state, store) = deletion_store("idempotent");
    plant_session(&store, "a", "hello");
    plant_session(&store, "b", "more");

    store.begin_delete("a").unwrap();
    store.begin_delete("a").unwrap();
    store.begin_delete("b").unwrap();
    assert_eq!(
        store.pending_deletions().unwrap(),
        vec!["a".to_owned(), "b".to_owned()],
        "pending deletions are sorted"
    );

    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);
    assert_eq!(
        store.finish_delete("a").unwrap(),
        DeleteStatus::Complete,
        "the second finish has no tombstone and removes nothing new"
    );
    assert!(!state.join("sessions/a.json").exists());
    assert!(!state.join("lifecycle/session-a.tombstone").exists());
    assert_eq!(store.pending_deletions().unwrap(), vec!["b".to_owned()]);
    assert_eq!(store.finish_delete("b").unwrap(), DeleteStatus::Complete);
    let _ = std::fs::remove_dir_all(state);
}
#[test]
fn finish_without_a_tombstone_deletes_nothing() {
    let (state, store) = deletion_store("no-tombstone");
    plant_session(&store, "a", "hello");
    let session_path = state.join("sessions/a.json");
    let before = std::fs::read(&session_path).unwrap();

    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);

    assert!(session_path.is_file(), "a bare id removes nothing");
    assert_eq!(std::fs::read(&session_path).unwrap(), before);
    assert!(!store.is_deleted("a").unwrap());
    let _ = std::fs::remove_dir_all(state);
}
#[test]
fn a_corrupt_tombstone_fails_closed() {
    let (state, store) = deletion_store("corrupt-tombstone");
    plant_session(&store, "a", "hello");
    let session_path = state.join("sessions/a.json");
    let before = std::fs::read(&session_path).unwrap();
    let lifecycle = state.join("lifecycle");
    std::fs::create_dir_all(&lifecycle).unwrap();
    std::fs::write(lifecycle.join("session-a.tombstone"), "not json at all").unwrap();

    assert_eq!(store.is_deleted("a").unwrap_err(), StoreError::Invalid);
    assert_eq!(store.finish_delete("a").unwrap_err(), StoreError::Invalid);
    assert_eq!(
        std::fs::read(&session_path).unwrap(),
        before,
        "the session data is untouched by a refused finish"
    );
    let _ = std::fs::remove_dir_all(state);
}
