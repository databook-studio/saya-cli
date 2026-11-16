/// Dispatch returns the replay task at once (nothing ran, nothing pushed),
/// and while the worker runs the loop keeps ticking: a poll applies nothing,
/// keys still edit the input, and the completion lands only on a later tick.
#[test]
fn replay_runs_in_background_and_loop_keeps_ticking() {
    // Dispatch: the adapter hands back a task immediately.
    let mut transcript = Transcript::new();
    let runtime = unused_runtime();
    let store = SqliteStateStore::new(PathBuf::new());
    let outcome = run_investigation(
        &mut transcript,
        &runtime,
        &store,
        RenderFormat::Text,
        &run_command("recent-orders"),
        &None,
    );
    let Some(Dispatch::ReplayTask(task)) = outcome else {
        panic!("a run dispatches a replay task");
    };
    assert_eq!(task.id, "recent-orders");
    assert!(
        transcript.blocks().is_empty(),
        "dispatch runs nothing inline: {:?}",
        transcript.blocks()
    );

    // The loop: a gated worker — the replay is in flight, so the app is busy.
    let mut app = idle_app();
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(temp_dir("ticking").as_path());
    let (gate, body) = gated(done(0, "the replay result", Some(replay())));
    put_running_replay(
        &mut app,
        spawn_with(WorkerPermit::for_tests(), body),
        "recent-orders",
    );

    // While the worker waits, polls apply nothing and keys keep working.
    tick_workers(&mut app, &store, &mut state);
    handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(
        app.input.text(),
        "x",
        "keys still work while the replay runs"
    );
    assert!(app.is_busy(), "a running replay reads as busy");

    // Release: the completion lands on a later tick, and the loop stays
    // prompt until it does.
    gate.release.send(()).expect("release the replay worker");
    gate.handed
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker handed off its outcome");
    tick_until(&mut app, &store, &mut state, |app| {
        app.replay_task.is_none()
    });
    // The typed replay drives the completion: a Table block like a direct
    // /sql result, never the captured CLI text (whose tabs a System block
    // strips).
    assert!(
        last_block(&app.transcript, BlockKind::Table).is_some_and(|text| text.contains("│ one │")),
        "the replay result renders as a table on completion"
    );
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|b| b.text.contains('\t')),
        "the captured tab-separated text is never pushed: {:?}",
        app.transcript.blocks()
    );
    assert!(!app.is_busy());
}

/// Esc detaches a running replay exactly like a SQL task: the receiver is
/// dropped, cancellation is requested, the message stays honest (may still
/// be running server-side, result discarded), and the late completion
/// changes nothing.
#[test]
fn detached_replay_late_completion_changes_nothing() {
    let mut app = idle_app();
    let (gate, body) = gated(done(0, "the replay result", Some(replay())));
    let rx = spawn_with(WorkerPermit::for_tests(), body);
    put_running_replay(&mut app, rx, "recent-orders");

    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        app.replay_task.is_none(),
        "detach clears the in-flight replay"
    );
    assert!(!app.is_busy());
    assert_eq!(
        app.request.started, None,
        "the bar's status fields are released"
    );
    assert_eq!(app.request.activity, None);
    let message = last_block(&app.transcript, BlockKind::System).expect("detach is said");
    assert!(
        message.contains("Cancellation requested; the investigation recent-orders was detached"),
        "the detach message names the cancellation: {message}"
    );
    assert!(
        message.contains("may still be running on the server") && message.contains("discarded"),
        "the detach message is honest about the server-side query: {message}"
    );
    assert!(
        message.contains("elapsed"),
        "the detach message still reports the elapsed time: {message}"
    );

    // The worker finishes late; its outcome lands on a dropped channel.
    gate.release.send(()).expect("release the replay worker");
    gate.handed
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker handed off (to a dropped receiver)");
    let mut state = SessionState::new("test", None, "model");
    tick_workers(
        &mut app,
        &session_store(temp_dir("detach").as_path()),
        &mut state,
    );

    // Nothing changed: the transcript holds only the detach message, and the
    // capture and selectable query are untouched.
    assert!(
        app.transcript.blocks().len() == 1,
        "a late completion after detach must not touch the transcript: {:?}",
        app.transcript.blocks()
    );
    assert!(app.captured.is_none(), "a detached replay never captures");
    assert!(app.last_query.is_none(), "a detached replay never promotes");
}
