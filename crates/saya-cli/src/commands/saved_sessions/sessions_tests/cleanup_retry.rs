#[cfg(unix)]
#[test]
fn pending_cleanup_is_reported_and_a_retry_converges() {
    use std::os::unix::fs::PermissionsExt;
    let sessions = scratch("pending-cleanup");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("a.json"), "{}").unwrap();
    let nested = sessions.join("a").join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("keep"), "x").unwrap();
    // Read+execute, no write: children inside cannot be unlinked, so the
    // cleanup cannot finish on this attempt.
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o500)).unwrap();
    let _restore = RestorePermissions(nested.clone());
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, "a", None);
    match &outcome {
        DeleteOutcome::Pending { remaining } => assert!(*remaining >= 1, "{outcome:?}"),
        other => panic!("expected Pending, got {other:?}"),
    }
    assert!(
        base.join("lifecycle").join("session-a.tombstone").exists(),
        "the tombstone is durable outside the sessions tree"
    );
    // Permissions restored: the same command converges.
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
    let outcome = delete_saved_session(&store, &sessions, "a", None);
    assert_eq!(outcome, DeleteOutcome::Deleted, "{outcome:?}");
    assert!(!sessions.join("a").exists());
    assert!(!sessions.join("a.json").exists());
    assert!(
        !base.join("lifecycle").join("session-a.tombstone").exists(),
        "a complete delete removes the tombstone"
    );
    let _ = std::fs::remove_dir_all(base);
}
