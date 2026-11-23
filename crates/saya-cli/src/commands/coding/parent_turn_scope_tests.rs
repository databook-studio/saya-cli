//! Every core turn runs on its own stop signal (phase-4 unit 4.0a).
//!
//! A turn's token is a child of the run's: cancelling the turn — what an
//! attached surface's Esc does — ends that turn and leaves the run taking
//! turns, while cancelling the run still stops the turn it is running.

#![cfg(unix)]

use super::super::super::CodingError;
use super::super::TurnInputs;
use super::super::parent_turn_fixture::in_place;
use super::super::parent_turn_provider::Scripted;
use super::{RUN, until};
use saya_agent::NoopEventSink;
use std::sync::Arc;

/// Test 1 — cancelling turn 1's own stop signal ends turn 1, and the run admits
/// turn 2 on the same epoch and the one lease.
#[tokio::test]
async fn a_cancelled_turn_ends_and_the_run_takes_the_next_one() {
    let provider = Scripted::start();
    provider.hold_first();
    let fixture = in_place("turn-scope", &provider.endpoint).await;
    let epoch = fixture.job.epoch;
    let running = {
        let job = Arc::clone(&fixture.job);
        tokio::spawn(async move {
            job.run_open(&NoopEventSink, TurnInputs::default())
                .await
                .map(|_| ())
        })
    };
    until(|| provider.arrived() >= 1).await;

    let turn = fixture
        .job
        .turn_cancellation()
        .expect("the running turn states its own stop signal");
    turn.cancel();
    let cancelled = tokio::time::timeout(RUN, running)
        .await
        .expect("the cancelled turn must settle")
        .expect("the turn's task must not panic");
    provider.release();
    let Err(cancelled) = cancelled else {
        panic!("a cancelled turn does not answer");
    };
    assert!(
        matches!(&cancelled, CodingError::Failed(failure) if failure.class == saya_types::CodingFailureClass::Cancelled),
        "the turn ended on its own stop signal, not on a provider failure: {cancelled:?}"
    );
    assert_eq!(
        fixture.job.cancellation.reason(),
        None,
        "the turn's stop stayed in the turn and never reached the run"
    );

    let second = tokio::time::timeout(
        RUN,
        fixture.job.run_next_turn(&NoopEventSink, "second message"),
    )
    .await
    .expect("the further turn must settle");
    assert!(
        second.is_ok(),
        "the run keeps taking turns after one was cancelled: {second:?}"
    );
    assert_eq!(
        fixture.job.epoch, epoch,
        "the further turn is another turn of the same epoch"
    );
    assert!(
        !fixture.job.holds_folder_lease(),
        "and takes no folder lease: this run states no write scope"
    );
}

/// A stop of the run is still the turn's too: the child follows its parent.
#[tokio::test]
async fn a_stop_of_the_run_still_stops_the_turn_it_is_running() {
    let provider = Scripted::start();
    provider.hold_first();
    let fixture = in_place("turn-scope-run", &provider.endpoint).await;
    let running = {
        let job = Arc::clone(&fixture.job);
        tokio::spawn(async move {
            job.run_open(&NoopEventSink, TurnInputs::default())
                .await
                .is_err()
        })
    };
    until(|| provider.arrived() >= 1).await;

    fixture.job.cancellation.cancel();
    let stopped = tokio::time::timeout(RUN, running)
        .await
        .expect("the stopped turn must settle")
        .expect("the turn's task must not panic");
    provider.release();
    assert!(
        stopped,
        "a stop of the run ends the turn that was running on it"
    );
}
