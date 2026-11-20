//! A root turn is one message of one open run, not one whole run.
//
//! Phase 3 unit 3-A: a message after the previous turn ended normally is
//! another turn of the SAME job — same epoch, one lease, no consent prompt,
//! no workspace re-open — while a managed-checkout job, a finished run and a
//! turn that is already running are refused.

#![cfg(unix)]

use super::super::{CodingError, NativeCodingJob};
use super::parent_turn_fixture::{in_place, managed};
use super::parent_turn_provider::Scripted;
use saya_agent::NoopEventSink;
use std::{sync::Arc, time::Duration};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

async fn first_turn(job: &Arc<NativeCodingJob>) -> Result<(), CodingError> {
    let result = tokio::time::timeout(RUN, job.run_open(&NoopEventSink))
        .await
        .expect("the first turn must settle");
    result.map(|_| ())
}

async fn next_turn(job: &Arc<NativeCodingJob>, prompt: &str) -> Result<(), CodingError> {
    let result = tokio::time::timeout(RUN, job.run_next_turn(prompt))
        .await
        .expect("the further turn must settle");
    result.map(|_| ())
}

async fn approvals(job: &NativeCodingJob) -> usize {
    job.application
        .store
        .list_coding_approval_requests(job.job_id(), None, 100)
        .await
        .expect("the job's approval audit reads")
        .items
        .len()
}

/// Waits, bounded, for `condition`: a scripted provider sees a request only
/// once the turn that sent it is under way.
async fn until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "the scripted turn never reached the provider"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn a_second_message_is_a_second_turn_of_the_same_open_run() {
    let provider = Scripted::start();
    let fixture = in_place("second-message", &provider.endpoint).await;
    let epoch = fixture.job.epoch;
    let before = approvals(&fixture.job).await;
    assert_eq!(before, 1, "the job's own consent is its one approval");

    let first = first_turn(&fixture.job).await;
    assert!(first.is_ok(), "the first turn must run: {first:?}");
    assert!(
        fixture.job._in_place_lease.is_some(),
        "the open in-place job holds its one folder lease"
    );

    let second = next_turn(&fixture.job, "second message").await;
    assert!(second.is_ok(), "the second turn must run: {second:?}");

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        2,
        "each turn sends exactly one provider request: {bodies:?}"
    );
    assert!(
        bodies[0].contains("first message"),
        "the first request carries the job's task: {}",
        bodies[0]
    );
    assert!(
        bodies[1].contains("second message"),
        "the second request carries the new message: {}",
        bodies[1]
    );
    assert_eq!(
        fixture.job.epoch, epoch,
        "a further turn is another turn of the same epoch, not a new one"
    );
    assert!(
        fixture.job._in_place_lease.is_some(),
        "the further turn ran on the lease the run already held, not a re-taken one"
    );
    assert_eq!(
        approvals(&fixture.job).await,
        before,
        "a further turn asks no consent"
    );
}

#[tokio::test]
async fn a_turn_while_a_turn_is_running_is_busy() {
    let provider = Scripted::start();
    provider.hold_first();
    let fixture = in_place("busy", &provider.endpoint).await;
    let running = {
        let job = Arc::clone(&fixture.job);
        tokio::spawn(async move { job.run_open(&NoopEventSink).await })
    };
    until(|| provider.arrived() >= 1).await;

    let refused = fixture.job.run_next_turn("second message").await;
    assert!(
        matches!(refused, Err(CodingError::Busy)),
        "a turn while another is running must be refused Busy: {refused:?}"
    );

    provider.release();
    let held = tokio::time::timeout(RUN, running)
        .await
        .expect("the held turn must settle")
        .expect("the held turn's task must not panic");
    assert!(held.is_ok(), "the held turn must still finish: {held:?}");
}

#[tokio::test]
async fn a_turn_after_the_run_finished_is_refused() {
    let provider = Scripted::start();
    let fixture = in_place("finished", &provider.endpoint).await;
    let run = tokio::time::timeout(RUN, fixture.job.run(&NoopEventSink))
        .await
        .expect("the single run must settle");
    assert!(run.is_ok(), "the single run must finish: {run:?}");

    let refused = fixture.job.run_next_turn("too late").await;
    let Err(error) = refused else {
        panic!("a finished run must take no further turn: {refused:?}");
    };
    assert!(
        matches!(error, CodingError::NotResumable { .. }),
        "a finished run is refused as a run that cannot be continued: {error:?}"
    );
    assert!(
        error.to_string().contains("finished its run"),
        "the refusal names the finished run: {error}"
    );
}

#[tokio::test]
async fn a_managed_checkout_job_takes_no_further_turn() {
    let fixture = managed("turn-managed").await;
    let refused = fixture.job.run_next_turn("second message").await;
    let Err(error) = refused else {
        panic!("a managed-checkout job must take no further turn: {refused:?}");
    };
    assert!(
        matches!(error, CodingError::NotResumable { .. }),
        "a managed-checkout job is refused as a run that cannot be continued: {error:?}"
    );
    assert!(
        error.to_string().contains("managed checkout"),
        "the refusal names the managed checkout: {error}"
    );
}
