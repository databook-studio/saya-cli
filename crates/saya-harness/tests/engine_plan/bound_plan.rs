use super::*;

/// A valid plan binds as the model proposed it, and the episode driving step
/// *n* sees only step *n*'s capabilities.
#[tokio::test]
async fn a_valid_plan_binds_and_step_n_sees_only_step_n_s_capabilities() {
    // Step 0 may not write the workspace, step 1 may; the run's approval
    // grants workspace-write, and each step narrows it.
    let plan_text = plan_json(&format!(
        "{},{}",
        step_json("read the manifest", "{}", None, None),
        step_json(
            "write the report",
            r#"{"workspace_write": true}"#,
            None,
            None
        )
    ));
    let planner = ScriptedPlanner::new(vec![plan_text]);
    let plan_driver = PlanDriver::new(&planner, planner_request("produce the report"));
    let mut approval = Capabilities::default();
    approval.workspace_write = true;

    let plan = plan_driver
        .propose(&approval, &Budgets::default())
        .await
        .unwrap();

    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].goal, "read the manifest");
    assert!(!plan.steps[0].capabilities.workspace_write);
    assert!(plan.steps[1].capabilities.workspace_write);

    // Driving the bound plan through the merged episode driver: each step's
    // episode is narrowed to that step's capabilities.
    let run = approved_run("bind").await;
    let episode = ScriptedEpisode::new(vec!["observed", "written"]);
    // One toolset per step, all carrying the run universe — the per-step
    // narrowing is what this test proves.
    let tools: Arc<dyn ToolExecutor> = Arc::new(NoTools);
    let definitions = vec![
        def("probe", LocalStateEffect::None),
        def("workspace_writer", LocalStateEffect::WriteWorkspace),
    ];
    let toolsets: Vec<StepToolset> = (0..2)
        .map(|_| StepToolset {
            executor: Arc::clone(&tools),
            definitions: definitions.clone(),
        })
        .collect();
    let allow = AllowApproval;
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
    );
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &episode,
            approval: &allow,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        saya_harness::engine::EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
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

    let definitions: Vec<Vec<String>> = episode
        .requests()
        .iter()
        .map(|request| {
            request
                .tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        definitions,
        vec![
            vec!["probe".to_string()],
            vec!["probe".to_string(), "workspace_writer".to_string()],
        ],
        "step n must see only step n's capabilities"
    );
    assert_eq!(sink.state(), RunState::Completed);

    let _ = fs::remove_dir_all(run.root);
}
