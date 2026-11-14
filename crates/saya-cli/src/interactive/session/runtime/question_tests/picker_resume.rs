use super::*;

/// The real picker resume surface: `tick_resume` is the loop tick the TUI
/// session picker arms (`overlays.pending_resume`, set exactly as
/// `application/picker.rs` does), and it drives the real loader plus the
/// shared `reacquire` seam. With a pending scope present beforehand, a
/// successful resume cancels the old token; a refused swap (the resumed
/// id's lock is held) leaves the pending question untouched.
#[test]
fn the_tui_picker_resume_abandons_the_question_on_success_and_keeps_it_on_failure() {
    // `SAYA_HISTORY=off` keeps `App::new` from reading the user's real
    // input history: the fixture touches no file outside its root.
    unsafe { std::env::set_var("SAYA_HISTORY", "off") };

    // A successful resume abandons the pending question.
    let root = temp_sessions_root_named("tui-resume");
    let runtime = test_runtime();
    let store = FsSessionStore::new(root.join("store"));
    let mut app = App::new(
        Vec::new(),
        Arc::new(runtime.clone()),
        SqliteStateStore::new(PathBuf::new()),
        Arc::new(SessionUniverse::empty()),
    );
    let mut session = SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "tui-resume-live",
        ApprovalPolicy::Ask,
        &root.join("sessions"),
    )
    .expect("the live session acquires");
    let mut state = SessionState::new("tui-resume-live", None, "test-model");
    let pending = session
        .begin_question(&runtime)
        .expect("the pending question begins");
    session.settle_question(&pending, true);
    block_on(store.save(SessionState::new("resumed-ok", None, "test-model").redacted()))
        .expect("the record saves");
    app.overlays.pending_resume = Some("resumed-ok".into());
    tick_resume(&mut app, &store, &mut state, &runtime, &mut session);
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Resumed session resumed-ok")),
        "the picker resume said it resumed, not {:?}",
        app.transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        pending.cancellation().is_cancelled(),
        "the successful resume cancelled the old question's token"
    );
    drop(session);
    drop(app);
    drop(store);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(root);

    // A refused resume (the resumed id's lock is held) leaves it untouched.
    let root = temp_sessions_root_named("tui-resume-held");
    let runtime = test_runtime();
    let store = FsSessionStore::new(root.join("store"));
    let mut app = App::new(
        Vec::new(),
        Arc::new(runtime.clone()),
        SqliteStateStore::new(PathBuf::new()),
        Arc::new(SessionUniverse::empty()),
    );
    let mut session = SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "tui-resume-held-live",
        ApprovalPolicy::Ask,
        &root.join("sessions"),
    )
    .expect("the live session acquires");
    let blocker = SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "resumed-held",
        ApprovalPolicy::Ask,
        &root.join("sessions"),
    )
    .expect("the blocker session acquires");
    let mut state = SessionState::new("tui-resume-held-live", None, "test-model");
    let pending = session
        .begin_question(&runtime)
        .expect("the pending question begins");
    session.settle_question(&pending, true);
    block_on(store.save(SessionState::new("resumed-held", None, "test-model").redacted()))
        .expect("the record saves");
    app.overlays.pending_resume = Some("resumed-held".into());
    tick_resume(&mut app, &store, &mut state, &runtime, &mut session);
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("already running")),
        "the refused resume said why, not {:?}",
        app.transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        !pending.cancellation().is_cancelled(),
        "the refused resume left the pending question's token untouched"
    );
    let again = session
        .begin_question(&runtime)
        .expect("the question re-begins after the refused resume");
    assert!(
        again.is_same_question(&pending),
        "the refused resume left the question pending: the next begin re-begins the same question"
    );
    drop(blocker);
    drop(session);
    drop(app);
    drop(store);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(root);
}
