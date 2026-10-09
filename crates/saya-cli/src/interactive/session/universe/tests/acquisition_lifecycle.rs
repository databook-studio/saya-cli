/// A second process on a live session's id refuses, naming the holder pid;
/// the pid lockfile is the same one runs use, at `sessions/<id>/lock`.
#[test]
fn a_second_acquisition_of_a_live_session_refuses() {
    let project = worktree("lock");
    let runtime = session_runtime(None);
    let first = crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-1",
        saya_agent::ApprovalPolicy::Ask,
        &crate::interactive::session::lifecycle::paths::default_session_dir(),
    )
    .expect("the first holder acquires");
    let error = crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-1",
        saya_agent::ApprovalPolicy::Ask,
        &crate::interactive::session::lifecycle::paths::default_session_dir(),
    )
    .map(|_: crate::interactive::session::runtime::SessionRuntime| ())
    .expect_err("a live holder refuses");
    assert!(
        error.contains("already running") && error.contains("pid"),
        "the refusal names the holder: {error}"
    );
    drop(first);
    // The released lock is reclaimable.
    crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-1",
        saya_agent::ApprovalPolicy::Ask,
        &crate::interactive::session::lifecycle::paths::default_session_dir(),
    )
    .expect("the lock is reclaimable after release");
    // Two different sessions on the same project both hold: no project lock.
    crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-2",
        saya_agent::ApprovalPolicy::Ask,
        &crate::interactive::session::lifecycle::paths::default_session_dir(),
    )
    .expect("a different session on the same project acquires");
    let _ = fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------------------
// deletion tombstones gate acquisition
// ---------------------------------------------------------------------------

/// A tombstoned id refuses acquisition BEFORE the state dir is touched: a
/// deleted or being-deleted session cannot be resurrected by an open
/// attempt, and its `sessions/<id>/` stays absent.
#[test]
fn a_deleted_session_cannot_be_acquired_and_its_state_dir_is_not_recreated() {
    let root = temp_dir("tombstone-acquire");
    let sessions_root = root.join("sessions");
    let runtime = session_runtime(None);
    let store = saya_store::FsSessionStore::new(sessions_root.clone());
    store
        .begin_delete("tombstoned-id")
        .expect("the tombstone is written");
    let error = crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "tombstoned-id",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .map(|_: crate::interactive::session::runtime::SessionRuntime| ())
    .expect_err("a tombstoned id refuses acquisition");
    assert!(
        error.contains("was deleted"),
        "the refusal names the deleted session: {error}"
    );
    assert!(
        !sessions_root.join("tombstoned-id").exists(),
        "acquisition must not recreate the deleted session's state dir"
    );
    let _ = fs::remove_dir_all(&root);
}
