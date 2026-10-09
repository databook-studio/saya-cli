/// While a replay runs, a second replay or /sql submitted is queued (the
/// `is_busy` gate), a task reaching the dispatch handler is refused by the
/// backstop guard — in both directions — and Esc detaches. A second replay
/// is never silently dropped or started over the first.
#[test]
fn second_replay_while_running_is_refused_or_queued() {
    let mut app = idle_app();
    let (gate, body) = gated(done(0, "the replay result", None));
    let rx = spawn_with(WorkerPermit::for_tests(), body);
    put_running_replay(&mut app, rx, "recent-orders");

    // A second replay submitted while one runs is queued, not dispatched.
    app.input.set_text("/investigation run other-one");
    app.submit();
    assert_eq!(
        app.pending.as_deref(),
        Some("/investigation run other-one"),
        "a second replay is queued behind the running one"
    );
    // A second /sql is queued the same way (one slot: it replaces the queue).
    app.input.set_text("/sql SELECT 1");
    app.submit();
    assert_eq!(app.pending.as_deref(), Some("/sql SELECT 1"));
    assert!(
        app.replay_task.is_some(),
        "queueing never disturbed the running replay"
    );

    // The backstop guard refuses a task reaching the handler — both ways.
    assert!(matches!(
        app.admit_second_replay(),
        SecondSqlDecision::Reject(_)
    ));
    assert!(matches!(
        app.admit_second_sql(),
        SecondSqlDecision::Reject(_)
    ));

    // Esc detaches through the real key path.
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.replay_task.is_none());

    // Ctrl+C detaches too, before reaching the quit arming.
    let (gate2, body2) = gated(done(0, "the replay result", None));
    put_running_replay(
        &mut app,
        spawn_with(WorkerPermit::for_tests(), body2),
        "recent-orders",
    );
    handle_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(
        app.replay_task.is_none(),
        "Ctrl+C detaches a running replay"
    );
    assert!(!app.ctrl_c_armed, "detach is not a quit arming");
    assert!(!app.should_quit);
    let _ = gate.release.send(());
    let _ = gate2.release.send(());
}

/// The replay is visible: the status bar shows the spinner and names the
/// investigation — "running investigation <id>" — never "thinking".
#[test]
fn running_replay_is_visible_in_the_status_bar() {
    const SPINNER_CHARS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let mut app = idle_app();
    let started = Instant::now();
    let (gate, body) = gated(done(0, "the replay result", None));
    app.replay_task = Some((
        spawn_with(WorkerPermit::for_tests(), body),
        replay_task("recent-orders"),
        started,
    ));
    app.request.started = Some(started);
    app.request.activity = Some("investigation recent-orders".into());
    let status = StatusView {
        profile: "local".into(),
        included: Vec::new(),
        model: "qwen".into(),
        approval_mode: "read-only".into(),
        agent_mode: "build".into(),
        workspace_root: None,
        sharing_on: true,
        host_composed: false,
        denied_programs: Vec::new(),
        task_summary: None,
    };
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| crate::interactive::tui::ui::draw(frame, &app, &status))
        .expect("draw completes");
    let buffer = format!("{}", terminal.backend());
    assert!(
        buffer.contains("running investigation recent-orders"),
        "the bar names the running investigation:\n{buffer}"
    );
    assert!(
        !buffer.contains("thinking"),
        "a replay must not be labelled 'thinking':\n{buffer}"
    );
    assert!(
        SPINNER_CHARS.iter().any(|c| buffer.contains(c)),
        "a spinner frame should be present:\n{buffer}"
    );
    let _ = gate.release.send(());
}
