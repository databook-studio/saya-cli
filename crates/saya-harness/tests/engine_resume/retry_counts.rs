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
        vec![Some(0), Some(0)],
        "the resume's journaled closing row and the fresh attempt's own \
         zero batch stand; the unobserved batch contributed nothing"
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

/// The second crash the closing row makes resumable: a Retry resume of a
/// pending batch journals the closing batch row, so when the resumed
/// attempt admits its own request and dies again, the journal reads admit,
/// batch, admit — evidence `from_journal` accepts — and a second Retry
/// resume reconciles the new pending batch instead of refusing the run as
/// invalid evidence forever. The plan carries three turns because the
/// scenario charges two admitted requests across the two crashes and the
/// second resume's fresh attempt needs one turn of headroom to complete.
#[tokio::test]
async fn a_retry_resume_journals_the_closing_batch_row_so_a_second_crash_resumes() {
    use futures_util::FutureExt;

    let plan = RunPlan::new(vec![bounded_step("bounded retry step", 3, Some(1))]).unwrap();
    let run = crashed_run(
        "retry-pending-second-crash",
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
    // The first crash lands right after the resume journals its closing
    // row: the wire dies on the first usage row the resume appends, the
    // append itself already durable — the same real-fault seam the
    // completion-wire-interruption test uses.
    let mut inputs = run.inputs();
    inputs.incomplete_policy = IncompletePolicy::Retry;
    inputs.journal_wire = Some(Arc::new(|event: &RunEvent| {
        if matches!(
            event,
            RunEvent::Usage {
                turns: None,
                tool_calls: Some(0),
                ..
            }
        ) {
            panic!("interrupt after the resume's closing batch row");
        }
    }));
    let _interrupted = std::panic::AssertUnwindSafe(resume(&run.run_dir, inputs))
        .catch_unwind()
        .await;

    // (i) the closing row stands in the journal, after the admission the
    // crash left, as the journal's last row.
    let events = Journal::open(&run.run_dir).read().unwrap();
    let admission = events
        .iter()
        .position(|event| *event == count_row(Some(1), None))
        .expect("the crashed admission is journaled");
    let closing = events
        .iter()
        .position(|event| *event == count_row(None, Some(0)))
        .expect("the resume journals the closing batch row");
    assert!(
        closing > admission,
        "the closing row must follow the admission it settles"
    );
    assert_eq!(
        events.last(),
        Some(&count_row(None, Some(0))),
        "the journal ends with the closing row: the resumed attempt never got past it"
    );

    // (ii) the second crash: the fresh attempt admits its own request and
    // dies before its response. The journal now reads admit, batch, admit.
    Journal::open(&run.run_dir)
        .append(&count_row(Some(1), None))
        .unwrap();
    let mut second = run.inputs();
    second.incomplete_policy = IncompletePolicy::Retry;

    let outcome = resume(&run.run_dir, second).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        },
        "the reconciled journal resumes instead of refusing the run forever"
    );
    assert_eq!(run.stubs.provider.count(), 1, "one fresh attempt");
}
