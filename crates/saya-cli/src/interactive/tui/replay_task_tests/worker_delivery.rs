/// The production worker end to end: it runs the shared typed operation
/// (`run_investigation_outcome`) on its own thread with the output captured
/// there, and the loop's completion captures the typed replay — source
/// saved investigation, scope full — against a real SQLite profile. Every
/// root is composed: the investigations root rides the runtime (resolved
/// from the process-env map `load_with_sources` takes, never the live
/// environment) and the state store rides the dispatched task, so no
/// environment variable is mutated anywhere.
#[tokio::test]
async fn replay_worker_runs_the_shared_operation_and_delivers_the_outcome() {
    // The real worker holds one process-wide permit for its whole lifetime,
    // so this test holds the permit lock across that lifetime — the cap
    // tests must never see this worker's slot mid-held-phase.
    let _permits = test_permit_lock_async().await;
    let root = temp_dir("worker-end-to-end");
    for dir in ["investigations", "config-home", "home"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    let database = root.join("data.sqlite3");
    std::fs::write(&database, b"").unwrap();
    std::fs::write(
        root.join("connections.toml"),
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    // The composed runtime: the investigations root is resolved at
    // composition from the process-env map this loader takes — the test's
    // private root — and the state store is composed beside it.
    let process = std::collections::BTreeMap::from([(
        "SAYA_INVESTIGATIONS_DIR".to_string(),
        root.join("investigations").display().to_string(),
    )]);
    let options = crate::cli::GlobalOptions {
        connections: Some(root.join("connections.toml")),
        ..Default::default()
    };
    let runtime = std::sync::Arc::new(
        crate::config::runtime::load_with_sources(
            &options,
            root.join("config-home").as_path(),
            root.join("config-home").as_path(),
            process,
        )
        .expect("the test runtime loads"),
    );
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));

    // Save one investigation through the shared operation first.
    crate::commands::capture_output_start();
    let outcome = crate::commands::run_investigation_outcome(
        InvestigationCommand::Save {
            name: "Recent orders".into(),
            description: None,
            sql: Some("SELECT 1 AS one".into()),
            file: None,
            connection: Some("local".into()),
            param_specs: Vec::new(),
        },
        &runtime,
        RenderFormat::Text,
        false,
        &state_db,
    )
    .await
    .expect("the save succeeds");
    let (out, err) = crate::commands::capture_output_take();
    assert_eq!(outcome.code, 0, "save stderr: {err}");
    let id = out
        .lines()
        .next()
        .expect("save prints the id")
        .trim()
        .to_string();

    // The real background replay: dispatch through the loop path. The app's
    // runtime is the test runtime and the task carries the session's state
    // store — the worker replays against the roots the dispatch composed,
    // not a dummy.
    let mut app = idle_app();
    app.runtime = std::sync::Arc::clone(&runtime);
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(&root);
    app.start_replay(ReplayTask {
        id: id.clone(),
        command: run_command(&id),
        format: RenderFormat::Text,
        state_db: state_db.clone(),
        cancellation: saya_agent::CancellationToken::new(),
    });
    assert!(app.is_busy(), "the replay is in flight");
    assert_eq!(
        app.request.activity.as_deref(),
        Some(format!("investigation {id}").as_str()),
        "the spinner names the investigation"
    );

    tick_until(&mut app, &store, &mut state, |app| {
        app.replay_task.is_none()
    });

    let captured = app.captured.as_ref().expect("the replay is captured");
    assert!(matches!(
        captured.evidence.source,
        EvidenceSource::SavedInvestigation { .. }
    ));
    assert_eq!(captured.evidence.scope, ResultScope::Full);
    let last_query = app.last_query.as_ref().expect("the replay is selectable");
    assert_eq!(last_query.sql, "SELECT 1 AS one");
    assert_eq!(last_query.connection.as_deref(), Some("local"));
    let block = last_block(&app.transcript, BlockKind::Table).expect("output pushed");
    assert!(
        block.contains("│ one │"),
        "the shared operation's result renders as a table: {block}"
    );
    assert!(
        block.contains("saved investigation: local"),
        "the shared operation's evidence line is in the output: {block}"
    );
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|b| b.text.contains('\t')),
        "the captured tab-separated CLI text is never pushed: {:?}",
        app.transcript.blocks()
    );

    let _ = std::fs::remove_dir_all(root);
}
