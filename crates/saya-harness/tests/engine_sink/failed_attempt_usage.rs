//! The failed-attempt receipt battery, end to end: one test drives the
//! whole claim — a successful turn and a collection-limit failure each
//! journal exactly their actual settled receipt, the executor never runs,
//! and a resume carries the journaled total past the token ceiling without
//! fabricating another receipt.
//!
//! The test itself stays here as the arrange; its phases live beside it:
//! `fixtures` (the scripted provider and no-op executor), `attempts` (the
//! three bounded provider attempts), `receipts` (the journal assertions),
//! and `resume` (the resumed run pausing on carried tokens).

#[path = "failed_attempt_usage/fixtures.rs"]
mod fixtures;

#[path = "failed_attempt_usage/attempts.rs"]
mod attempts;

#[path = "failed_attempt_usage/receipts.rs"]
mod receipts;

#[path = "failed_attempt_usage/resume.rs"]
mod resume;

use std::time::Instant;

use saya_agent::CancellationToken;
use saya_harness::engine::{EngineEventSink, RunState, SinkBudgets, TransitionEvent, UsageTotals};
use saya_harness::journal::Journal;
use saya_types::RunEvent;

use attempts::drive_bounded_attempts;
use fixtures::{NoTools, scripted_receipt_provider};
use receipts::assert_actual_receipts;
use resume::resume_pauses_on_carried_tokens;

#[tokio::test]
async fn actual_failed_attempt_receipt_is_journaled_and_carried_by_resume() {
    let root = super::temp_root("failed-attempt-usage");
    let (store, run_id) =
        super::seeded_store(&root, "failed-attempt-usage", super::RunStatus::Approved).await;
    let run_dir = root.join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    journal
        .append(&RunEvent::PlanApproved { scopes: None })
        .unwrap();
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        journal.clone(),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(40),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    journal.append(&RunEvent::StepStarted { step: 0 }).unwrap();

    let provider = scripted_receipt_provider();
    let tools = NoTools::default();
    let cancellation = CancellationToken::default();

    drive_bounded_attempts(&provider, &tools, &sink, cancellation).await;
    assert_actual_receipts(&journal, &tools, &provider);
    resume_pauses_on_carried_tokens(&root, &run_dir, &journal, &run_id, store, &provider, &sink)
        .await;
    let _ = std::fs::remove_dir_all(root);
}
