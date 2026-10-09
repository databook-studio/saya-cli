use super::*;

/// EpisodeDriver cannot enter provider/tool work when StepStarted cannot be
/// appended. Use a real journal filesystem fault rather than a test seam.
#[tokio::test]
async fn a_step_start_append_failure_prevents_episode_work() {
    let run = crashed_run(
        "start-append-failure",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Approved,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let journal_path = run.run_dir.join("events.ndjson");
    fs::remove_file(&journal_path).unwrap();
    fs::create_dir(&journal_path).unwrap();
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
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
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &run.stubs.provider,
            approval: &run.stubs.approval,
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal,
        },
        request(),
        bounds(),
    );

    assert!(
        driver
            .run_step(&sink, &run.plan, 0, &workspace(&run.root))
            .await
            .is_err()
    );
    assert_eq!(run.stubs.provider.count(), 0);
    assert!(
        RunStore::list_steps(&*run.store, &run.run_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_step_start_mirror_refusal_prevents_episode_work() {
    let run = crashed_run(
        "start-mirror-failure",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Approved,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let store: Arc<dyn RunStore> = Arc::new(RefuseStepMirrorStore {
        inner: (*run.store).clone(),
        refuse_status: RunStepStatus::Running,
    });
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
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
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &run.stubs.provider,
            approval: &run.stubs.approval,
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store,
            journal,
        },
        request(),
        bounds(),
    );

    assert!(
        driver
            .run_step(&sink, &run.plan, 0, &workspace(&run.root))
            .await
            .is_err()
    );
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(run.tool_calls.load(Ordering::Relaxed), 0);
    assert!(run.journal().contains(&RunEvent::StepStarted { step: 0 }));
    assert!(
        RunStore::list_steps(&*run.store, &run.run_id)
            .await
            .unwrap()
            .is_empty()
    );
}
