use super::*;

/// A synced StepCompleted followed by a wire interruption precedes the step
/// mirror. Resume must trust that durable prefix, continue the next step, and
/// never call the provider for the completed step again.
#[tokio::test]
async fn a_durable_completed_prefix_survives_mirror_interruption_on_resume() {
    use futures_util::FutureExt;

    let run = crashed_run(
        "completion-wire-interruption",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![step("completed step"), step("next step")]).unwrap(),
    )
    .await;
    let journal = Journal::open(&run.run_dir).with_wire(Arc::new(|event| {
        if matches!(event, RunEvent::StepCompleted { step: 0 }) {
            panic!("interrupt after the durable completion append");
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
    let interrupted =
        std::panic::AssertUnwindSafe(driver.run_step(&sink, &run.plan, 0, &workspace(&run.root)))
            .catch_unwind()
            .await;

    assert!(interrupted.is_err());
    assert!(run.journal().contains(&RunEvent::StepCompleted { step: 0 }));
    assert_eq!(
        RunStore::list_steps(&*run.store, &run.run_id)
            .await
            .unwrap()[0]
            .status,
        RunStepStatus::Running,
        "the injected interruption happened before the step mirror"
    );
    assert_eq!(run.stubs.provider.count(), 1);

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 1,
            state: RunState::Completed
        }
    );
    assert_eq!(run.stubs.provider.count(), 2);
    let events = run.journal();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 0 }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepCompleted { step: 0 }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 1 }))
            .count(),
        1
    );
}
