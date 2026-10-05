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

#[tokio::test]
async fn a_failed_request_settles_as_an_empty_batch_and_the_step_retries() {
    let run = approved_run("count-settle").await;
    let provider = ScriptProvider::new(vec![Turn::Fail, Turn::Answer("recovered")]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let mut budget = Budgets::default();
    budget.turns = Some(3);
    let plan = RunPlan::new(vec![step_with_budget(
        "fail once then answer",
        Some(budget),
    )])
    .unwrap();

    driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap();

    assert_eq!(provider.requests().len(), 2);
    assert_eq!(
        count_pairs_for_step(&Journal::open(&run.run_dir).read().unwrap(), 0),
        vec![
            (Some(0), Some(0)),
            (Some(1), None),
            (None, Some(0)),
            (Some(1), None),
            (None, Some(0)),
        ],
        "the failed request closes as an empty batch before the retry admits"
    );
    assert_eq!(sink.state(), RunState::Completed);
    let _ = fs::remove_dir_all(run.root);
}

#[tokio::test]
async fn a_provider_failing_every_attempt_exhausts_normally_not_count_evidence() {
    let run = approved_run("count-exhaust").await;
    let provider = ScriptProvider::new(vec![Turn::Fail, Turn::Fail, Turn::Fail]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let mut budget = Budgets::default();
    budget.turns = Some(3);
    let plan = RunPlan::new(vec![step_with_budget("always fails", Some(budget))]).unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert!(
        matches!(error, EpisodeError::StepExhausted { .. }),
        "the normal exhausted path, never CountEvidence: {error:?}"
    );
    assert_eq!(sink.state(), RunState::Paused);
    assert_eq!(
        count_pairs_for_step(&Journal::open(&run.run_dir).read().unwrap(), 0),
        vec![
            (Some(0), Some(0)),
            (Some(1), None),
            (None, Some(0)),
            (Some(1), None),
            (None, Some(0)),
            (Some(1), None),
            (None, Some(0)),
        ],
        "each failed attempt's request closes as an empty batch"
    );
    let _ = fs::remove_dir_all(run.root);
}

fn usage_pair(turns: Option<u64>, tool_calls: Option<u64>) -> RunEvent {
    RunEvent::Usage {
        endpoint: "orchestrator".into(),
        tokens: None,
        turns,
        tool_calls,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

#[tokio::test]
async fn the_settled_journal_replays_not_pending_with_requests_carried() {
    use saya_harness::engine::{IncompletePolicy, ResumeError, ResumeRun, resume};

    let run = approved_run("count-replay").await;
    run.store
        .set_run_status(&run.run_id, RunStatus::Executing, None)
        .await
        .unwrap();
    let journal = Journal::open(&run.run_dir);
    for event in [
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::StepStarted { step: 0 },
        usage_pair(Some(0), Some(0)),
        usage_pair(Some(1), None),
        usage_pair(None, Some(0)),
        RunEvent::StepFailed { step: 0 },
        RunEvent::StepStarted { step: 0 },
        usage_pair(Some(1), None),
        usage_pair(None, Some(0)),
    ] {
        journal.append(&event).unwrap();
    }
    let provider = ScriptProvider::new(vec![Turn::Answer("must not be admitted")]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let mut budget = Budgets::default();
    budget.turns = Some(2);
    let plan = RunPlan::new(vec![step_with_budget("the settled step", Some(budget))]).unwrap();

    let error = resume(
        &run.run_dir,
        ResumeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            plan,
            incomplete_policy: IncompletePolicy::Retry,
            workspace: run.workspace(),
            collaborators: EpisodeCollaborators {
                provider: &provider,
                approval: &approval,
                toolsets: &toolsets,
                cancellation: CancellationToken::default(),
            },
            request: EpisodeRequest {
                model: "mock-model".into(),
                profile_names: Vec::new(),
                memory_allows_candidate_writes: false,
            },
            bounds: bounds(),
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            journal_wire: None,
            agent_stream: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        !matches!(error, ResumeError::CountEvidence { .. }),
        "the settled journal replays not-pending, never a count refusal: {error:?}"
    );
    assert!(
        matches!(
            error,
            ResumeError::Episode {
                source: EpisodeError::BudgetStopped { .. }
            }
        ),
        "the run continued on the replayed counts and stopped on its spent grant: {error:?}"
    );
    assert!(
        provider.requests().is_empty(),
        "the journal carried requests == 2: the turns=2 grant admits nothing more"
    );
    let _ = fs::remove_dir_all(run.root);
}
