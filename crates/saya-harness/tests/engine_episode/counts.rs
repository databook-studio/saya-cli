use super::*;
use saya_types::RunEvent;

fn count_pairs(events: &[RunEvent]) -> Vec<(Option<u64>, Option<u64>)> {
    events
        .iter()
        .filter_map(|event| match event {
            RunEvent::Usage {
                tokens: None,
                turns,
                tool_calls,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
                ..
            } if turns.is_some() || tool_calls.is_some() => Some((*turns, *tool_calls)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_new_step_initializes_a_fresh_count_grant() {
    let run = approved_run("count-next-step").await;
    let provider = ScriptProvider::new(vec![Turn::Answer("step one"), Turn::Answer("step two")]);
    let approval = AllowApproval;
    let tools: Arc<dyn ToolExecutor> = Arc::new(RecordingTools::default());
    let toolsets = vec![
        toolset(Arc::clone(&tools), Vec::new()),
        toolset(tools, Vec::new()),
    ];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let plan = RunPlan::new(vec![
        step_with_budget("first independent grant", Some(one_request_budget())),
        step_with_budget("second independent grant", Some(one_request_budget())),
    ])
    .unwrap();

    for step in 0..plan.steps.len() {
        driver
            .run_step(&sink, &plan, step, &run.workspace())
            .await
            .unwrap();
    }

    let events = Journal::open(&run.run_dir).read().unwrap();
    let expected = vec![(Some(0), Some(0)), (Some(1), None), (None, Some(0))];
    assert_eq!(count_pairs_for_step(&events, 0), expected);
    assert_eq!(count_pairs_for_step(&events, 1), expected);
    assert_eq!(provider.requests().len(), 2);
    assert_eq!(sink.state(), RunState::Completed);
    let _ = fs::remove_dir_all(run.root);
}

fn one_request_budget() -> Budgets {
    let mut budget = Budgets::default();
    budget.turns = Some(1);
    budget
}

fn count_pairs_for_step(events: &[RunEvent], step: usize) -> Vec<(Option<u64>, Option<u64>)> {
    let Some(start) = events
        .iter()
        .position(|event| matches!(event, RunEvent::StepStarted { step: found } if *found == step))
    else {
        return Vec::new();
    };
    events[start + 1..]
        .iter()
        .take_while(
            |event| !matches!(event, RunEvent::StepStarted { step: found } if *found != step),
        )
        .filter_map(|event| match event {
            RunEvent::Usage {
                tokens: None,
                turns,
                tool_calls,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
                ..
            } if turns.is_some() || tool_calls.is_some() => Some((*turns, *tool_calls)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn retry_uses_only_remaining_step_request_and_tool_grants() {
    let run = approved_run("count-retry").await;
    let first = call("probe");
    let duplicate = ToolCall {
        id: first.id.clone(),
        name: "probe".into(),
        arguments: serde_json::json!({}),
    };
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![first, duplicate]),
        Turn::Tools(vec![call("probe")]),
        Turn::Answer("must not be admitted"),
    ]);
    let tools = Arc::new(RecordingTools::default());
    let executor: Arc<dyn ToolExecutor> = tools.clone();
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        executor,
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let mut budget = Budgets::default();
    budget.turns = Some(2);
    budget.tool_calls = Some(1);
    let plan = RunPlan::new(vec![step_with_budget(
        "keep the remaining grant",
        Some(budget),
    )])
    .unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert!(matches!(error, EpisodeError::BudgetStopped { .. }));
    assert_eq!(
        provider.requests().len(),
        2,
        "the third request exceeds the step grant"
    );
    assert!(
        tools.calls().is_empty(),
        "an exhausted tool grant starts no effect"
    );
    assert_eq!(sink.state(), RunState::Paused);
    assert_eq!(
        count_pairs(&Journal::open(&run.run_dir).read().unwrap()),
        vec![
            (Some(0), Some(0)),
            (Some(1), None),
            (None, Some(2)),
            (Some(1), None),
            (None, Some(1)),
        ],
        "the retry's source observations append to the same step totals"
    );
    let _ = fs::remove_dir_all(run.root);
}

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
