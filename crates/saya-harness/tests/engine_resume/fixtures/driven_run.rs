/// Drives a two-step run end to end on a fresh store: `RunStarted` claimed,
/// the plan approved through the sink, both steps driven. Its journal is
/// the fixture the crash-prefix test truncates.
async fn driven_run(label: &str) -> CrashedRun {
    let root = temp_root(label);
    let store = Arc::new(SqliteStateStore::new(root.join("state.sqlite3")));
    let run_id = RunId::parse(&format!("run-{label}")).unwrap();
    store
        .create_run(NewRun {
            id: run_id.clone(),
            capabilities: RunCapabilityFlags::default(),
            budgets: RunBudgets::default(),
        })
        .await
        .unwrap();
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    let stubs = Stubs::default();
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Planned,
        journal.clone(),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    sink.record(TransitionEvent::Approve { scopes: vec![] })
        .await
        .unwrap();
    let plan = two_step_plan();
    let no_tools = Arc::new(NoTools::default());
    let tool_calls = Arc::clone(&no_tools.calls);
    let executor: Arc<dyn ToolExecutor> = no_tools;
    let toolsets: Vec<StepToolset> = (0..plan.steps.len())
        .map(|_| StepToolset {
            executor: Arc::clone(&executor),
            definitions: Vec::new(),
        })
        .collect();
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &stubs.provider,
            approval: &stubs.approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run_id.clone(),
            store: store.clone(),
            journal: journal.clone(),
        },
        request(),
        bounds(),
    );
    driver
        .run_step(&sink, &plan, 0, &workspace(&root))
        .await
        .unwrap();
    driver
        .run_step(&sink, &plan, 1, &workspace(&root))
        .await
        .unwrap();
    CrashedRun {
        root,
        run_dir,
        store,
        run_id,
        stubs,
        plan,
        tool_calls,
        toolsets,
    }
}
