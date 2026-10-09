//! The second crash a Retry resume survives: the closing batch row the
//! resume journals keeps a twice-crashed run resumable.

use super::*;

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
