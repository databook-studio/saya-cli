use super::fixtures::*;
use super::*;

/// Approving the plan once does not produce a per-tool-call prompt
/// afterwards: the steps complete with a tool that requires approval, and
/// the prompting decider is never asked.
#[tokio::test]
async fn approving_the_plan_once_does_not_prompt_per_tool_call() {
    let run = planned_run("no-per-call").await;
    let plan = saya_types::RunPlan::new(vec![
        StepSpec::new(
            "survey the schema",
            workspace_write_scopes(),
            None,
            Vec::new(),
            None,
        )
        .unwrap(),
        StepSpec::new(
            "survey the rows",
            workspace_write_scopes(),
            None,
            Vec::new(),
            None,
        )
        .unwrap(),
    ])
    .unwrap();
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Planned,
        journal.clone(),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    // The one approval interaction: the plan, decided once.
    sink.record(TransitionEvent::Approve { scopes: vec![] })
        .await
        .unwrap();

    let episode = ScriptedEpisode::new(vec![
        Turn::Tools(vec![call("sql_probe")]),
        Turn::Answer("first step done"),
        Turn::Tools(vec![call("sql_probe")]),
        Turn::Answer("second step done"),
    ]);
    // One executor shared by both steps' toolsets: the call log the test
    // asserts on is the one the loop wrote across both episodes.
    let tools = Arc::new(RecordingTools::default());
    let definitions = vec![approved_read_tool()];
    let toolsets: Vec<StepToolset> = (0..2)
        .map(|_| StepToolset {
            executor: tools.clone(),
            definitions: definitions.clone(),
        })
        .collect();
    let approval = CountingApproval {
        prompts: AtomicUsize::new(0),
    };
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &episode,
            approval: &approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: journal.clone(),
        },
        saya_harness::engine::EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        ManifestBounds {
            max_files: 8,
            max_file_bytes: 64 * 1024,
        },
    );
    let workspace = run.workspace();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();
    driver.run_step(&sink, &plan, 1, &workspace).await.unwrap();

    // The tool calls really ran — the zero below is meaningful.
    assert_eq!(
        tools.calls.lock().unwrap().clone(),
        vec!["sql_probe".to_string(), "sql_probe".to_string()],
        "both steps' tool calls executed under the plan approval"
    );
    assert_eq!(
        approval.prompted(),
        0,
        "an approved plan must not produce per-tool-call prompts"
    );
    assert_eq!(sink.state(), RunState::Completed);

    let journal_text = fs::read_to_string(run.run_dir.join("events.ndjson")).unwrap();
    assert_eq!(
        journal_text.matches("plan_approved").count(),
        1,
        "exactly one approval interaction is on the durable record: {journal_text}"
    );
    let _ = fs::remove_dir_all(run.root);
}

/// The needs-approval refusal names the missing scope, so the composition
/// root's message can say what was missing rather than refuse generically.
#[tokio::test]
async fn the_needs_approval_refusal_names_the_missing_scope() {
    let asking = plan_json(&step_json(
        "fetch the report",
        r#"{"fetch": {"destinations": [{"scheme": "https", "host": "example.com"}]}}"#,
    ));
    let planner = ScriptedPlanner::new(vec![asking; 3]);
    let driver = PlanDriver::new(&planner, planner_request("fetch"));

    let error = driver
        .propose(&workspace_write_scopes(), &Budgets::default())
        .await
        .unwrap_err();

    let last = match &error {
        PlanError::Exhausted { last, .. } => last,
        other => panic!("expected exhaustion, got {other:?}"),
    };
    assert!(
        last.to_string().contains("fetch:https+example.com"),
        "the refusal must name the missing scope: {last}"
    );
}
