use super::*;

#[tokio::test]
async fn accounting_append_failure_stops_before_the_next_tool_and_keeps_journal_error() {
    let run = approved_run("accounting-journal-failure").await;
    let journal_path = run.run_dir.join("events.ndjson");
    let kept_journal = run.run_dir.join("events-before-failure.ndjson");
    let provider = ReceiptThenTool {
        journal_path,
        kept_journal: kept_journal.clone(),
        calls: AtomicUsize::new(0),
        cancel_first: None,
    };
    let executor = Arc::new(RecordingTools::default());
    let tools = vec![toolset(
        executor.clone(),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let cancellation = CancellationToken::default();
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &provider,
            approval: &AllowApproval,
            toolsets: &tools,
            cancellation: cancellation.clone(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    );
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_cancellation(cancellation.clone());
    let plan = RunPlan::new(vec![step(
        "account usage before tool",
        Capabilities::default(),
    )])
    .unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure),
        "a missing usage receipt stops the same agent/tool owner before another effect"
    );
    assert!(
        executor.calls().is_empty(),
        "the tool must not run after receipt I/O failed"
    );
    assert!(matches!(
        &error,
        EpisodeError::OwnerFailureDiagnostic { diagnostic: error, .. }
            if matches!(error.as_ref(), saya_harness::engine::EngineSinkError::Journal { .. })
    ));
    let retained = fs::read_to_string(kept_journal).unwrap();
    let retained_events = retained
        .lines()
        .map(|line| serde_json::from_str::<RunEvent>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        retained_events,
        vec![RunEvent::StepStarted { step: 0 }],
        "the pre-failure journal remains intact and no successful usage receipt is fabricated"
    );
    assert!(
        fs::metadata(run.run_dir.join("events.ndjson"))
            .unwrap()
            .is_dir()
    );
    let _ = fs::remove_dir_all(run.root);
}

#[tokio::test]
async fn user_first_cancel_write_failure_keeps_nonterminal_state() {
    let run = approved_run("user-accounting-journal-failure").await;
    let journal_path = run.run_dir.join("events.ndjson");
    let kept_journal = run.run_dir.join("events-before-failure.ndjson");
    let cancellation = CancellationToken::default();
    let provider = ReceiptThenTool {
        journal_path,
        kept_journal: kept_journal.clone(),
        calls: AtomicUsize::new(0),
        cancel_first: Some(cancellation.clone()),
    };
    let executor = Arc::new(RecordingTools::default());
    let tools = vec![toolset(
        executor.clone(),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let driver = deadline_driver(&run, &provider, &tools, cancellation.clone());
    let sink = deadline_sink(&run, cancellation.clone(), Duration::from_secs(2));
    let plan = RunPlan::new(vec![step(
        "user stop before usage append failure",
        Capabilities::default(),
    )])
    .unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert_eq!(cancellation.reason(), Some(CancellationReason::User));
    assert!(executor.calls().is_empty());
    assert!(matches!(
        &error,
        EpisodeError::Transition {
            source: saya_harness::engine::EngineSinkError::Journal { .. }
        }
    ));
    assert_eq!(sink.state(), RunState::Executing);
    let retained = fs::read_to_string(kept_journal).unwrap();
    let retained_events = retained
        .lines()
        .map(|line| serde_json::from_str::<RunEvent>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(retained_events.contains(&RunEvent::StepStarted { step: 0 }));
    assert!(!retained_events.iter().any(|event| matches!(
        event,
        RunEvent::Usage { .. } | RunEvent::Cancelled | RunEvent::Completed
    )));
    let _ = fs::remove_dir_all(run.root);
}

#[tokio::test]
async fn approved_clock_observation_failure_preserves_diagnostic_without_source_work() {
    let run = approved_run("approved-clock-write-failure").await;
    fs::create_dir(run.run_dir.join("events.ndjson")).unwrap();
    let provider = ScriptProvider::new(Vec::new());
    let tools = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let cancellation = CancellationToken::default();
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &provider,
            approval: &AllowApproval,
            toolsets: &tools,
            cancellation: cancellation.clone(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    );
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_elapsed_clock(ElapsedClock::arm_now(Duration::from_secs(30)).unwrap())
    .with_cancellation(cancellation.clone());
    let plan = RunPlan::new(vec![step(
        "clock must persist before work",
        Capabilities::default(),
    )])
    .unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert!(matches!(error, EpisodeError::SinkDiagnostic { .. }));
    assert!(provider.requests().is_empty(), "the source is not polled");
    assert_eq!(sink.state(), RunState::Approved);
    assert!(matches!(
        sink.take_diagnostic().as_deref(),
        Some(saya_harness::engine::EngineSinkError::Journal { .. })
    ));
    let _ = fs::remove_dir_all(run.root);
}
