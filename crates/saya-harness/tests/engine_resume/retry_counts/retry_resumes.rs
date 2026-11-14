//! The operator's explicit Retry: the same journals the conservative
//! policy refuses resume as a fresh attempt — a pending request stays
//! charged while its unobserved batch contributes nothing, and a
//! never-initialized step starts from its own initialization row.

use super::*;

/// The same journal under Retry: the operator reconciled the possible
/// effects, so the step runs a fresh attempt — the pending request stays
/// charged and its unobserved tool batch contributes nothing.
#[tokio::test]
async fn retry_resumes_a_pending_batch_with_the_request_charged() {
    let plan = RunPlan::new(vec![bounded_step("bounded retry step", 2, Some(1))]).unwrap();
    let run = crashed_run(
        "retry-pending-resumed",
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
    let mut inputs = run.inputs();
    inputs.incomplete_policy = IncompletePolicy::Retry;

    let outcome = resume(&run.run_dir, inputs).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        }
    );
    assert_eq!(run.stubs.provider.count(), 1, "one fresh attempt");
    let events = run.journal();
    let admitted_rows = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RunEvent::Usage {
                    turns: Some(1),
                    tool_calls: None,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        admitted_rows, 2,
        "the charged row and the fresh attempt's own admit"
    );
    let batch_rows: Vec<Option<u64>> = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::Usage {
                turns: None,
                tool_calls,
                ..
            } => Some(*tool_calls),
            _ => None,
        })
        .collect();
    assert_eq!(
        batch_rows,
        vec![Some(0), Some(0)],
        "the resume's journaled closing row and the fresh attempt's own \
         zero batch stand; the unobserved batch contributed nothing"
    );
}

/// The same journal under Retry: nothing was charged before the crash, so
/// the fresh attempt starts from its own initialization row and admits its
/// own request.
#[tokio::test]
async fn retry_resumes_a_missing_initialization_from_fresh_counts() {
    let plan = RunPlan::new(vec![bounded_step("bounded retry step", 2, None)]).unwrap();
    let run = crashed_run(
        "retry-init-resumed",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let mut inputs = run.inputs();
    inputs.incomplete_policy = IncompletePolicy::Retry;

    let outcome = resume(&run.run_dir, inputs).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        }
    );
    assert_eq!(run.stubs.provider.count(), 1, "one fresh attempt");
    let events = run.journal();
    assert!(events.contains(&count_row(Some(0), Some(0))));
    assert!(events.contains(&count_row(Some(1), None)));
}
