//! The conservative policy's pinned refusals: a pending batch left by the
//! crash between `RequestAdmitted` and its response, and a step started
//! without any initialization the policy can trust — both refused before
//! any work starts.

use super::*;

/// The crash between `RequestAdmitted` and its response: the conservative
/// policy refuses — pinned, the safe reading of unknown unobserved calls.
#[tokio::test]
async fn the_conservative_policy_refuses_a_pending_batch() {
    let plan = RunPlan::new(vec![bounded_step("bounded retry step", 2, Some(1))]).unwrap();
    let run = crashed_run(
        "retry-pending-refused",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            count_row(Some(0), Some(0)),
            count_row(Some(1), None),
        ],
        RunStatus::Executing,
        plan,
    )
    .await;

    let error = resume(&run.run_dir, run.inputs()).await.unwrap_err();

    assert!(matches!(
        error,
        ResumeError::CountEvidence {
            step: 0,
            source: AnsweringCountError::PendingBatch
        }
    ));
    assert_eq!(
        run.stubs.provider.count(),
        0,
        "refused evidence starts no work"
    );
}

/// The crash between `StepStarted` and the first count rows: no
/// initialization the conservative policy can trust — refusal pinned.
#[tokio::test]
async fn the_conservative_policy_refuses_a_missing_initialization() {
    let plan = RunPlan::new(vec![bounded_step("bounded retry step", 2, None)]).unwrap();
    let run = crashed_run(
        "retry-init-refused",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;

    let error = resume(&run.run_dir, run.inputs()).await.unwrap_err();

    assert!(matches!(
        error,
        ResumeError::CountEvidence {
            step: 0,
            source: AnsweringCountError::MissingInitialization
        }
    ));
    assert_eq!(
        run.stubs.provider.count(),
        0,
        "refused evidence starts no work"
    );
}
