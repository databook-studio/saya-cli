//! The run a rollover ends and the run it prepares: while the session is ready
//! a change rolls at once, and while a turn runs the change waits for it.
//!
//! Each test drives the real job engine and the scripted provider, so what is
//! asserted is the request the next run actually sent, the durable state the
//! ended run left, and the record's link.

#![cfg(unix)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use saya_agent::NoopEventSink;
use saya_store::RunLink;

use super::ChatOutcome;
use super::compose_fixture;
use super::operator_front_fixture;
use super::rollover::{ContextChange, Ending, Rollover};
use super::rollover_fixture::{RUN, job_id, last_body, messages, next_run, open, transcript, turn};
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Test 1: while the session is ready, a model change closes run n and prepares
/// run n+1, whose first turn carries the transcript's pairs — and the record
/// links the new job.
#[tokio::test]
async fn a_change_while_ready_closes_the_run_and_seeds_the_next_one() {
    let provider = Scripted::start_with_script(vec![
        Reply::Text("first answer"),
        Reply::Text("second answer"),
        Reply::Text("the next run answered"),
    ]);
    let fixture = compose_fixture::in_place("rollover-ready", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let session = open(fixture, &front, "roll-ready").await;
    let mut transcript = transcript("roll-ready");

    for (prompt, outcome) in [
        ("first message", "first answer"),
        ("second message", "second answer"),
    ] {
        let answer = turn(session.run.job(), prompt).await;
        assert_eq!(answer, outcome, "the scripted turn answers");
        assert!(
            transcript.settle(prompt, &answer, &ChatOutcome::Answered),
            "an answered turn is recorded in the transcript"
        );
    }

    let rollover = Rollover::new(Arc::clone(&session.application));
    let rolled = rollover
        .roll(
            Ending::new(session.run),
            ContextChange::Model,
            next_run(front.front(), "roll-ready-2"),
            &mut transcript,
        )
        .await
        .expect("the rollover prepares the next run");

    // Run n is closed — and finalized, which happens once: the run's terminal
    // state and its outcome block are on disk, written by the rollover.
    let ended = session
        .application
        .status(&job_id("roll-ready"))
        .await
        .expect("run n is a job the application owns");
    assert_eq!(
        ended["state"], "completed",
        "run n is closed and finalized exactly once: {ended}"
    );

    // The link names the run that replaced it.
    let link = transcript
        .record()
        .run
        .clone()
        .expect("the rollover writes the link of the run it prepared");
    assert_eq!(
        link,
        RunLink {
            job: job_id("roll-ready-2"),
            epoch: rolled.job().epoch(),
        },
        "the record names the new job at the epoch the rollover prepared"
    );

    // Run n+1's first turn replays the transcript's pairs, in order, as its
    // history — and adds only the new prompt behind them.
    let answered = turn(rolled.job(), "third message").await;
    assert_eq!(answered, "the next run answered");
    let sent = messages(&last_body(&provider));
    assert_eq!(
        sent.len(),
        6,
        "the next run's first request is its system message, the transcript's two pairs and its prompt: {sent:?}"
    );
    assert_eq!(
        &sent[1..5],
        [
            ("user".to_owned(), "first message".to_owned()),
            ("assistant".to_owned(), "first answer".to_owned()),
            ("user".to_owned(), "second message".to_owned()),
            ("assistant".to_owned(), "second answer".to_owned()),
        ],
        "the next run's first turn carries the transcript's pairs as its history: {sent:?}"
    );
    assert!(
        sent[5].0 == "user" && sent[5].1.contains("third message"),
        "the new prompt follows the history: {sent:?}"
    );
}

/// Test 2: a change requested while a turn runs waits for it, and rolls once it
/// ended — carrying the answer that turn gave into the next run's seed.
#[tokio::test]
async fn a_change_while_busy_rolls_after_the_turn_ends() {
    let provider = Scripted::start();
    provider.hold_first();
    let fixture = compose_fixture::in_place("rollover-busy", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let session = open(fixture, &front, "roll-busy").await;
    let mut transcript = transcript("roll-busy");

    let running = {
        let job = Arc::clone(session.run.job());
        tokio::spawn(async move { job.run_next_turn(&NoopEventSink, "first message").await })
    };
    let deadline = Instant::now() + RUN;
    while provider.arrived() < 1 {
        assert!(
            Instant::now() < deadline,
            "the first turn must reach the provider"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let rollover = Rollover::new(Arc::clone(&session.application));
    rollover.request(
        ContextChange::Connection,
        next_run(front.front(), "roll-busy-2"),
    );
    assert!(
        rollover.waiting(),
        "a change requested during a turn is held, not dropped"
    );
    assert!(
        session
            .application
            .status(&job_id("roll-busy-2"))
            .await
            .is_err(),
        "nothing rolls while the turn in flight still owns the run"
    );

    provider.release();
    let answer = tokio::time::timeout(RUN, running)
        .await
        .expect("the held turn settles")
        .expect("the turn's task must not panic")
        .expect("the turn runs")
        .answer;
    assert_eq!(answer, "turn 0 answered");
    assert!(
        transcript.settle("first message", &answer, &ChatOutcome::Answered),
        "the answer the operator read is recorded"
    );

    let rolled = rollover
        .after_turn(Ending::new(session.run), &mut transcript)
        .await
        .expect("the held change rolls once the turn ended")
        .expect("a waiting change rolls");
    assert!(
        !rollover.waiting(),
        "the change that rolled is no longer waiting"
    );

    let answered = turn(rolled.job(), "second message").await;
    assert_eq!(answered, "turn 1 answered");
    let sent = messages(&last_body(&provider));
    assert_eq!(
        sent.len(),
        4,
        "the next run's first request is its system message, the answered pair and its prompt: {sent:?}"
    );
    assert_eq!(
        &sent[1..3],
        [
            ("user".to_owned(), "first message".to_owned()),
            ("assistant".to_owned(), "turn 0 answered".to_owned()),
        ],
        "the answer of the turn the change waited on is in the next run's seed: {sent:?}"
    );
    assert!(
        sent[3].0 == "user" && sent[3].1.contains("second message"),
        "the new prompt follows the seed: {sent:?}"
    );
}
