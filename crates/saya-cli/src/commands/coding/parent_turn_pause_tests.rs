//! A progress guard's stop reads as a pause on the turn that stopped, with the
//! guard's own message — not as a failure, and not as the run ending.
//!
//! A backstop is the other half, and it is the opposite reading: a runaway bound
//! the operator raises stays a failure. `turn_stop_tests` states that half.

#![cfg(unix)]

use super::super::TurnInputs;
use super::super::parent_turn_fixture::in_place;
use super::super::parent_turn_provider::{Reply, Scripted};
use saya_agent::NoopEventSink;
use std::time::Duration;

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// Test 4a — a no-progress guard stop on a turn reads as paused and names
/// `limits.job.no_progress_turns`; the turn keeps the answer it produced.
#[tokio::test]
async fn a_no_progress_guard_stop_reads_as_paused_with_its_own_message() {
    // Nine turns that change nothing: the guard stops the loop, and the turn
    // ends on the model's last answer rather than on a thrown-away one.
    let script = (0..8)
        .map(|_| Reply::FruitlessGrep)
        .chain([Reply::Text("nothing to report")])
        .collect();
    let provider = Scripted::start_with_script(script);
    let fixture = in_place("turn-pause", &provider.endpoint).await;

    let settled = tokio::time::timeout(
        RUN,
        fixture.job.run_open(&NoopEventSink, TurnInputs::default()),
    )
    .await
    .expect("the turn must settle")
    .expect("a guard stop keeps the turn's answer");
    assert_eq!(
        settled.answer, "nothing to report",
        "a pause keeps the answer the turn produced"
    );

    let stop = fixture
        .job
        .turn_stop()
        .expect("the turn states what stopped it");
    assert!(
        stop.paused(),
        "a progress guard's stop is a pause, not a failure: {stop:?}"
    );
    assert!(
        stop.message().contains("limits.job.no_progress_turns"),
        "the pause line names the key to raise: {}",
        stop.message()
    );
}
