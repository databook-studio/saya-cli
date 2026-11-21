//! A run opened without a turn: the first message is the run's first turn.
//!
//! A session prepares its run while the human types — the slot claimed, the
//! provider, toolset, hooks and lease held, the run open in this process — and
//! sends no request. The first message then arrives as `run_next_turn`, which
//! carries that message and not the job's stable `request.task` label.

use super::super::super::{CodingError, NativeCodingJob, live_runs};
use super::super::TurnInputs;
use super::super::parent_turn_fixture::in_place;
use super::super::parent_turn_provider::Scripted;
use super::{RUN, next_turn};
use saya_agent::NoopEventSink;
use serde_json::Value;

/// The label the fixture's job states: a real, stable name for the run, never
/// a message.
const LABEL: &str = "first message: read this folder";

/// The messages one provider request carried.
fn messages(body: &str) -> Vec<Value> {
    let body: Value = serde_json::from_str(body).expect("the provider request is JSON");
    body["messages"]
        .as_array()
        .cloned()
        .expect("the request states its messages")
}

fn role(message: &Value) -> &str {
    message["role"].as_str().expect("a message states its role")
}

fn text(message: &Value) -> &str {
    message["content"]
        .as_str()
        .expect("a replayed message is text")
}

/// The message the turn sent, as the core assembles it into the user turn: the
/// run's own message always comes last, after any untrusted context block the
/// turn states (the continuation snapshot every root turn carries), so the tail
/// after the last block's closing delimiter IS that message.
fn sent_prompt(content: &str) -> &str {
    const CLOSE: &str = "<<<CONTEXT_BLOCK_END>>>\n\n";
    content
        .rsplit_once(CLOSE)
        .map_or(content, |(_, prompt)| prompt)
}

/// Prepares the run, bounded: an open that never settles fails the test rather
/// than hanging it.
async fn prepare(job: &std::sync::Arc<NativeCodingJob>) -> Result<(), CodingError> {
    tokio::time::timeout(RUN, job.run_prepare(&NoopEventSink, TurnInputs::default()))
        .await
        .expect("the open must settle")
}

#[tokio::test]
async fn prepare_opens_without_a_turn() {
    let provider = Scripted::start();
    let fixture = in_place("prepare-open", &provider.endpoint).await;
    let epoch = fixture.job.epoch;

    let prepared = prepare(&fixture.job).await;
    assert!(prepared.is_ok(), "the run must open: {prepared:?}");

    assert!(
        fixture.job.turn_stop().is_none(),
        "no turn has ended on a run that ran none"
    );
    assert_eq!(
        provider.arrived(),
        0,
        "the open asks the provider nothing: {:?}",
        provider.bodies()
    );
    assert!(
        live_runs::run_is_open(fixture.job.job_id().as_str()),
        "the run is open in this process, so no verb may write to its stream"
    );
    assert_eq!(
        fixture.job.epoch, epoch,
        "the open takes no new epoch: it is the run, not a resume"
    );
    assert!(
        fixture.job._in_place_lease.is_none(),
        "the open takes no folder lease: a run with no write scope never holds one"
    );
}

#[tokio::test]
async fn first_message_is_a_next_turn() {
    let provider = Scripted::start();
    let fixture = in_place("prepare-first-message", &provider.endpoint).await;
    assert!(
        prepare(&fixture.job).await.is_ok(),
        "the run must open before its first message"
    );

    let first = next_turn(&fixture.job, "q").await;
    assert!(first.is_ok(), "the first message must run: {first:?}");

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        1,
        "the first message sends exactly one request: {bodies:?}"
    );
    let sent = messages(&bodies[0]);
    let asked = sent.last().expect("the request carries its own message");
    assert_eq!(role(asked), "user", "the turn's own message is last");
    assert_eq!(
        sent_prompt(text(asked)),
        "q",
        "the turn carries the message that was sent, in full and unaltered: {asked}"
    );
    assert!(
        sent.iter()
            .filter(|message| role(message) == "user")
            .all(|message| !text(message).contains(LABEL)),
        "the job's label is not a user message of the turn: {bodies:?}"
    );
}

#[tokio::test]
async fn prepare_twice_is_busy() {
    let provider = Scripted::start();
    let fixture = in_place("prepare-busy", &provider.endpoint).await;
    assert!(
        prepare(&fixture.job).await.is_ok(),
        "the run must open the first time"
    );

    let again = prepare(&fixture.job).await;
    assert!(
        matches!(again, Err(CodingError::Busy)),
        "a second open of the same run is refused Busy: {again:?}"
    );
    assert_eq!(
        provider.arrived(),
        0,
        "the refused open asks the provider nothing"
    );
}
