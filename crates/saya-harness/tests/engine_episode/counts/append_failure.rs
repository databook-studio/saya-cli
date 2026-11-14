use super::*;

#[tokio::test]
async fn count_append_failure_stops_before_provider_effects() {
    use saya_harness::engine::EngineSinkError;

    let run = approved_run("count-append-failure").await;
    let provider = ScriptProvider::new(vec![Turn::Answer("must not start")]);
    let approval = AllowApproval;
    let cancellation = CancellationToken::default();
    let toolsets = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let journal_path = run.run_dir.join("events.ndjson");
    let saved_path = run.run_dir.join("events.saved");
    let path_for_wire = journal_path.clone();
    let saved_for_wire = saved_path.clone();
    let journal = Journal::open(&run.run_dir).with_wire(Arc::new(move |event| {
        if matches!(event, RunEvent::StepStarted { step: 0 }) {
            fs::rename(&path_for_wire, &saved_for_wire).unwrap();
            fs::create_dir(&path_for_wire).unwrap();
        }
    }));
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
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &provider,
            approval: &approval,
            toolsets: &toolsets,
            cancellation,
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal,
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    );

    let mut budget = Budgets::default();
    budget.turns = Some(2);
    let plan = RunPlan::new(vec![step_with_budget(
        "persist admission first",
        Some(budget),
    )])
    .unwrap();
    let result = driver.run_step(&sink, &plan, 0, &run.workspace()).await;
    fs::remove_dir(&journal_path).unwrap();
    fs::rename(&saved_path, &journal_path).unwrap();

    assert!(
        matches!(
            &result,
            Err(EpisodeError::OwnerFailureDiagnostic {
                diagnostic,
                ..
            }) if matches!(diagnostic.as_ref(), EngineSinkError::Journal { .. })
        ),
        "count admission failure must remain diagnostic: {result:?}"
    );
    assert!(
        provider.requests().is_empty(),
        "the failed admission is before provider work"
    );
    assert_eq!(
        Journal::open(&run.run_dir).read().unwrap(),
        vec![RunEvent::StepStarted { step: 0 }],
        "a failed count append is not claimed as durable"
    );
    let _ = fs::remove_dir_all(run.root);
}
