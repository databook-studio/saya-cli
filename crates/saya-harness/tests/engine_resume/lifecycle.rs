use super::*;

/// A completed run resumes to "nothing to do": no step re-runs, no event is
/// re-journalled.
#[tokio::test]
async fn a_completed_run_resumes_to_nothing_to_do() {
    let run = driven_run("settled").await;
    let journal_before = run.journal();
    let requests_before = run.stubs.provider.count();

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Settled {
            state: RunState::Completed
        }
    );
    assert_eq!(
        run.stubs.provider.count(),
        requests_before,
        "a completed run must not re-run its last step"
    );
    assert_eq!(run.journal(), journal_before);
    let recorded = run.store.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(recorded.status, RunStatus::Completed);
}

/// A step executing at the crash restarts from its beginning — a fresh
/// attempt, not a mid-turn resume — and the restart is visible in the
/// journal as a second `StepStarted` for the same step.
#[tokio::test]
async fn a_step_executing_at_the_crash_restarts_from_its_start() {
    let plan = RunPlan::new(vec![step("the only step")]).unwrap();
    let run = crashed_run(
        "restart",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        }
    );
    assert_eq!(
        run.stubs.provider.count(),
        1,
        "the restarted step ran one fresh attempt from its start"
    );
    let events = run.journal();
    assert_eq!(
        events,
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::ProcessDeath
            },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
            RunEvent::Completed,
        ],
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 0 }))
            .count(),
        2,
        "the restart must be visible in the journal"
    );
}

#[tokio::test]
async fn a_second_engine_on_the_same_run_directory_is_refused() {
    let run = crashed_run(
        "locked",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let holder = RunLock::acquire(run.run_dir.join("lock")).unwrap();

    let error = resume(&run.run_dir, run.inputs()).await.unwrap_err();

    assert!(
        matches!(
            error,
            ResumeError::Lock {
                source: HarnessError::LockHeld { .. }
            }
        ),
        "the second engine must be refused by the run lock: {error:?}"
    );
    // The first engine releases the lock; the refusal was the lock alone.
    drop(holder);
    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();
    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        },
        "once the lock is released the resume must proceed"
    );
}
