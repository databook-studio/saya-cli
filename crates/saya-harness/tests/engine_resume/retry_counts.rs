//! Retry policy against half-written count evidence: the conservative
//! refusal of a pending batch and a missing initialization is pinned, and
//! the operator's explicit Retry lets the bounded step start a fresh
//! attempt from the counts the journal does hold.

use super::*;
use saya_harness::engine::{AnsweringCountError, IncompletePolicy};
use saya_types::Budgets;

fn bounded_step(goal: &str, turns: u64, tool_calls: Option<u64>) -> StepSpec {
    let mut budget = Budgets::default();
    budget.turns = Some(turns);
    budget.tool_calls = tool_calls;
    StepSpec::new(
        goal,
        Capabilities::default(),
        Some(budget),
        Vec::new(),
        None,
    )
    .unwrap()
}

/// One journaled count row, shaped exactly as the engine journals answering
/// observations: every figure the call did not report is absent.
fn count_row(turns: Option<u64>, tool_calls: Option<u64>) -> RunEvent {
    RunEvent::Usage {
        endpoint: "orchestrator".into(),
        tokens: None,
        turns,
        tool_calls,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

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
        vec![Some(0)],
        "only the fresh attempt's own zero batch stands; the unobserved \
         batch contributed nothing"
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
