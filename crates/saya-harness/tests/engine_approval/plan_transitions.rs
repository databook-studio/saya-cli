use super::fixtures::*;
use super::*;

/// A planned run refuses to begin a step until the plan is approved; the
/// approval goes through the sink's `TransitionEvent` path exactly once,
/// and a second approval is refused by the machine.
#[tokio::test]
async fn a_planned_run_refuses_to_begin_until_the_plan_is_approved_once() {
    let run = planned_run("gate").await;
    let plan = saya_types::RunPlan::new(vec![
        StepSpec::new(
            "survey the data",
            Capabilities::default(),
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
    let episode = ScriptedEpisode::new(vec![Turn::Answer("done")]);
    let tools: Arc<dyn ToolExecutor> = Arc::new(RecordingTools::default());
    let approval = CountingApproval {
        prompts: AtomicUsize::new(0),
    };
    let toolsets = vec![StepToolset {
        executor: tools,
        definitions: vec![],
    }];
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

    // Before approval there is nothing to run under.
    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            EpisodeError::NotRunnable {
                step: 0,
                state: RunState::Planned
            }
        ),
        "a planned run must refuse to begin: {error:?}"
    );

    // The one approval, through the transition path — never a direct write.
    sink.record(TransitionEvent::Approve { scopes: vec![] })
        .await
        .unwrap();
    assert_eq!(sink.state(), RunState::Approved);
    let recorded = RunStore::get_run(run.store.as_ref(), &run.run_id)
        .await
        .unwrap()
        .expect("the run is recorded");
    assert_eq!(recorded.status, RunStatus::Approved);

    // Approval is per plan, granted once: the machine refuses a second one.
    assert!(
        sink.record(TransitionEvent::Approve { scopes: vec![] })
            .await
            .is_err(),
        "approving an already-approved plan must be refused"
    );

    // Now the step runs and the plan completes.
    driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap();
    assert_eq!(sink.state(), RunState::Completed);

    let journal_text = fs::read_to_string(run.run_dir.join("events.ndjson")).unwrap();
    assert_eq!(
        journal_text.matches("plan_approved").count(),
        1,
        "exactly one approval is on the durable record: {journal_text}"
    );
    let _ = fs::remove_dir_all(run.root);
}

/// A plan asking for a scope the approved set does not hold is refused
/// rather than inheriting anything: the same approved set refuses it,
/// naming the missing scope; only an explicit widening binds. The engine
/// proposes once, before approval — there is no mid-run revision flow —
/// so this test drives the propose gate itself, twice.
#[tokio::test]
async fn a_reproposed_plan_asking_a_new_scope_is_refused_rather_than_inheriting() {
    let first = plan_json(&step_json(
        "write the report",
        r#"{"workspace_write": true}"#,
    ));
    let widening = plan_json(&step_json(
        "seed the scratch database",
        r#"{"scratch": true}"#,
    ));
    // The first proposal binds on one answer; the re-proposal is re-prompted
    // to the bound; the widened proposal binds on one answer.
    let planner = ScriptedPlanner::new(vec![
        first,
        widening.clone(),
        widening.clone(),
        widening,
        plan_json(&step_json(
            "seed the scratch database",
            r#"{"scratch": true}"#,
        )),
    ]);
    let driver = PlanDriver::new(&planner, planner_request("write, then seed"));

    let approved = workspace_write_scopes();
    let first_plan = driver
        .propose(&approved, &Budgets::default())
        .await
        .unwrap();
    assert_eq!(first_plan.steps.len(), 1, "the approved plan binds");

    // A re-proposal against the same approved set: the new scope does not
    // inherit.
    let error = driver
        .propose(&approved, &Budgets::default())
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::NeedsApproval { step: 0, scopes },
            } if scopes == &["scratch".to_string()]
        ),
        "the refusal must name the new scope: {error:?}"
    );

    // The approval widens explicitly; the re-proposed plan binds.
    let mut granted = workspace_write_scopes();
    granted.scratch = true;
    let reproposed = driver.propose(&granted, &Budgets::default()).await.unwrap();
    assert!(reproposed.steps[0].capabilities.scratch);
}
