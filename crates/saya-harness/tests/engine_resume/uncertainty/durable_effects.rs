use super::*;

#[tokio::test]
async fn a_completed_effect_survives_step_mirror_failure_without_replay() {
    let run = effect_run("completed-effect-mirror").await;
    let counter = Arc::new(AtomicUsize::new(0));
    let provider = EffectProvider(AtomicUsize::new(0));
    let store: Arc<dyn RunStore> = Arc::new(RefuseStepMirrorStore {
        inner: (*run.store).clone(),
        refuse_status: RunStepStatus::Done,
    });

    assert!(
        drive_effect(&run, 1, store, Arc::clone(&counter), &provider, None)
            .await
            .is_err()
    );
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert!(run.journal().contains(&RunEvent::StepCompleted { step: 1 }));

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert!(matches!(outcome, ResumeOutcome::Settled { .. }));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_effect_without_durable_completion_is_uncertain_until_explicit_retry() {
    for torn in [false, true] {
        let label = if torn { "torn-effect" } else { "absent-effect" };
        let run = effect_run(label).await;
        let counter = Arc::new(AtomicUsize::new(0));
        let provider = EffectProvider(AtomicUsize::new(0));
        let backup = Arc::new(Mutex::new(None));
        let observed = Arc::new(AtomicBool::new(false));
        let observer: Arc<dyn AgentEventSink> = Arc::new(InterruptAfterDelivery {
            run_dir: run.run_dir.clone(),
            backup: Arc::clone(&backup),
            observed_durable_start: Arc::clone(&observed),
        });

        let interrupted = drive_effect(
            &run,
            1,
            run.store.clone(),
            Arc::clone(&counter),
            &provider,
            Some(observer),
        )
        .await;
        assert!(
            interrupted.is_err(),
            "expected append interruption after effect {}, got {interrupted:?}; journal {:?}",
            counter.load(Ordering::SeqCst),
            run.journal()
        );
        restore_journal(&run, &backup);
        assert!(observed.load(Ordering::SeqCst));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        if torn {
            use std::io::Write;
            fs::OpenOptions::new()
                .append(true)
                .open(run.run_dir.join("events.ndjson"))
                .unwrap()
                .write_all(b"{\"type\":\"step_completed\"")
                .unwrap();
        }

        let retry_provider = EffectProvider(AtomicUsize::new(0));
        let retry_toolsets = effect_toolsets(Arc::clone(&counter));
        let mut inputs = run.inputs();
        inputs.collaborators.provider = &retry_provider;
        inputs.collaborators.toolsets = &retry_toolsets;
        let outcome = resume(&run.run_dir, inputs).await.unwrap();
        assert!(matches!(
            outcome,
            ResumeOutcome::IncompleteEffects { step: 1, .. }
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        let mut inputs = run.inputs();
        inputs.collaborators.provider = &retry_provider;
        inputs.collaborators.toolsets = &retry_toolsets;
        inputs.incomplete_policy = saya_harness::engine::IncompletePolicy::Retry;
        let retried = resume(&run.run_dir, inputs).await.unwrap();
        assert!(matches!(
            retried,
            ResumeOutcome::Resumed { first_step: 1, .. }
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert_eq!(retry_provider.0.load(Ordering::SeqCst), 2);
        let events = run.journal();
        assert_eq!(
            events
                .iter()
                .filter(|event| **event == RunEvent::StepStarted { step: 0 })
                .count(),
            1,
            "the durable prefix is never rerun"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| **event == RunEvent::StepStarted { step: 1 })
                .count(),
            2,
            "only the incomplete effect is retried"
        );
    }
}

#[tokio::test]
async fn later_incomplete_risky_steps_are_checked_before_any_suffix_work() {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let risky = StepSpec::new("later effect", capabilities, None, Vec::new(), None).unwrap();
    let plan = RunPlan::new(vec![step("earlier safe"), risky]).unwrap();
    let run = crashed_run(
        "later-risky-started",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 1 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert!(matches!(
        outcome,
        ResumeOutcome::IncompleteEffects { step: 1, .. }
    ));
    assert_eq!(run.stubs.provider.count(), 0);
    assert!(!run.journal().contains(&RunEvent::StepStarted { step: 0 }));
}

#[tokio::test]
async fn a_completed_nonprefix_step_is_never_replayed() {
    let plan = RunPlan::new(vec![
        step("not started"),
        step("completed out of order"),
        step("finish the plan"),
    ])
    .unwrap();
    let run = crashed_run(
        "nonprefix-completed",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Executing,
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
    driver
        .run_step(&sink, &run.plan, 1, &workspace(&run.root))
        .await
        .unwrap();
    assert_eq!(run.stubs.provider.count(), 1);

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert!(matches!(
        outcome,
        ResumeOutcome::Resumed { first_step: 0, .. }
    ));
    assert_eq!(run.stubs.provider.count(), 3);
    assert_eq!(
        run.journal()
            .iter()
            .filter(|event| **event == RunEvent::StepStarted { step: 1 })
            .count(),
        1
    );
}
