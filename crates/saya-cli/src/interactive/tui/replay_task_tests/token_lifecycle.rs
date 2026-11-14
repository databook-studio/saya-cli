/// Esc/Ctrl+C detach gives a replay the same real cancellation the direct-SQL
/// path has: the task's token is cancelled and the message says so, while it
/// stays honest that the query may still settle server-side.
#[test]
fn detaching_a_replay_cancels_its_token() {
    let mut app = idle_app();
    let (gate, body) = gated(done(0, "the replay result", None));
    put_running_replay(
        &mut app,
        spawn_with(WorkerPermit::for_tests(), body),
        "recent-orders",
    );
    let token = app
        .replay_task
        .as_ref()
        .expect("the replay is in flight")
        .1
        .cancellation
        .clone();

    app.detach_replay_task();
    assert!(
        app.replay_task.is_none(),
        "detach clears the in-flight replay"
    );
    assert!(
        token.is_cancelled(),
        "detach must cancel the replay's cancellation token"
    );
    let message = last_block(&app.transcript, BlockKind::System).expect("detach is said");
    assert!(
        message.contains("Cancellation requested; the investigation recent-orders was detached"),
        "the detach message names the cancellation: {message}"
    );
    assert!(
        message.contains("may still be running on the server") && message.contains("discarded"),
        "the detach message stays honest about the server-side query: {message}"
    );
    let _ = gate.release.send(());
}

/// A normally completed replay never cancels its token: only the detach arm
/// cancels; the poll/completion path stays untouched.
#[test]
fn a_completed_replay_does_not_cancel_its_token() {
    let mut app = idle_app();
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(temp_dir("completed-no-cancel").as_path());
    let (gate, body) = gated(done(0, "the replay result", Some(replay())));
    put_running_replay(
        &mut app,
        spawn_with(WorkerPermit::for_tests(), body),
        "recent-orders",
    );
    let token = app
        .replay_task
        .as_ref()
        .expect("in flight")
        .1
        .cancellation
        .clone();

    gate.release.send(()).expect("release the replay worker");
    gate.handed
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker handed off its outcome");
    tick_until(&mut app, &store, &mut state, |app| {
        app.replay_task.is_none()
    });
    assert!(
        last_block(&app.transcript, BlockKind::Table).is_some(),
        "the replay completed normally"
    );
    assert!(
        !token.is_cancelled(),
        "a completed replay must not cancel its token"
    );
}
