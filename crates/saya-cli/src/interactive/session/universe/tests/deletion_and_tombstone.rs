/// Deletion through the lifecycle owner: while a live runtime holds the id,
/// `delete_session` refuses (LockHeld); once the runtime drops, the deletion
/// completes and the id is FREE. That freedom is a recorded decision, not an
/// accident: a completed deletion removes the tombstone too, so nothing
/// remembers the old record and acquiring the same id again starts a FRESH
/// state dir and works.
#[test]
fn a_session_deleted_through_the_lifecycle_owner_cannot_be_reacquired() {
    use saya_store::SessionStore;

    let root = temp_dir("tombstone-lifecycle");
    let sessions_root = root.join("sessions");
    let runtime = session_runtime(None);
    let store = saya_store::FsSessionStore::new(sessions_root.clone());
    // A real saved record behind the id, so the deletion removes a live one.
    crate::interactive::session::lifecycle::resume::block_on(
        store.save(
            super::super::SessionState::new("lifecycle-deleted", None, "test-model").redacted(),
        ),
    )
    .expect("the session record saves");
    let live = crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lifecycle-deleted",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the live session acquires");
    let error = saya_harness::session_lifecycle::delete_session(
        &store,
        &sessions_root,
        "lifecycle-deleted",
    )
    .expect_err("deletion refuses a live holder");
    let error = format!("{error}");
    assert!(
        error.contains("held by a live process") && error.contains("pid"),
        "the refusal names the live holder: {error}"
    );
    drop(live);
    let status = saya_harness::session_lifecycle::delete_session(
        &store,
        &sessions_root,
        "lifecycle-deleted",
    )
    .expect("the deletion completes once the holder is gone");
    assert_eq!(status, saya_store::DeleteStatus::Complete);
    // DECISION: a completed deletion frees the id — the next acquisition of
    // it starts a FRESH state dir and succeeds.
    let fresh = crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lifecycle-deleted",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the id is free after a completed deletion: a fresh session acquires it");
    assert!(
        sessions_root.join("lifecycle-deleted").exists(),
        "the fresh acquisition created a fresh state dir"
    );
    drop(fresh);
    let _ = fs::remove_dir_all(&root);
}

/// The `/resume` seam (`reacquire`) refuses a tombstoned id the same way,
/// and the refused swap leaves the current session untouched: it still holds
/// its own id's lock, and its question lifecycle keeps working.
#[test]
fn reacquire_refuses_a_tombstoned_id() {
    let root = temp_dir("tombstone-reacquire");
    let sessions_root = root.join("sessions");
    let runtime = session_runtime(None);
    let store = saya_store::FsSessionStore::new(sessions_root.clone());
    let mut session = crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "reacquire-live",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the live session acquires");
    store
        .begin_delete("reacquire-tombstoned")
        .expect("the tombstone is written");
    let error = session
        .reacquire(
            &runtime,
            None,
            "reacquire-tombstoned",
            saya_agent::ApprovalPolicy::Ask,
        )
        .expect_err("a tombstoned id refuses the swap");
    assert!(
        error.contains("was deleted"),
        "the refusal names the deleted session: {error}"
    );
    // The old runtime is not replaced: it still holds its own id's lock.
    let again = crate::interactive::session::runtime::SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "reacquire-live",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .map(|_: crate::interactive::session::runtime::SessionRuntime| ())
    .expect_err("the old runtime still holds its lock");
    assert!(
        again.contains("already running") && again.contains("pid"),
        "the old runtime kept its lock: {again}"
    );
    let scope = session
        .begin_question(&runtime)
        .expect("the old runtime's question lifecycle still works");
    session.settle_question(&scope, false);
    drop(session);
    let _ = fs::remove_dir_all(&root);
}
