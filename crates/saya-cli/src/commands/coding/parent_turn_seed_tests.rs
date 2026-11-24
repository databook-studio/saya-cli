//! A prepared run begins with the conversation it was seeded with.
//!
//! A session hands the run it prepares the transcript of the run before it.
//! Those pairs are history of this run in exactly chat's shape and under chat's
//! one bound: they ride the turn's request as (prompt, answer) messages, they
//! stay at the front of every later turn, and past the run's context byte budget
//! the oldest of them are dropped. An empty seed carries no history at all.

use super::super::TurnInputs;
use super::super::parent_turn_fixture::{in_place, in_place_with_budget};
use super::super::parent_turn_provider::Scripted;
use super::{CodingError, RUN, next_turn};
use saya_agent::NoopEventSink;
use serde_json::Value;

/// The context byte budget the bounded case states: room for the newest pair,
/// not the oldest.
const BUDGET: usize = 12 * 1024;

/// One seeded (prompt, answer) pair, as a session hands its transcript over.
fn pair(prompt: String, answer: String) -> (String, String) {
    (prompt, answer)
}

/// A prompt or answer of exactly `bytes` characters, opening with `marker`.
fn padded(marker: &str, bytes: usize) -> String {
    format!("{marker} {}", "x".repeat(bytes - marker.len() - 1))
}

/// Opens the run with `seed`, bounded: an open that never settles fails the
/// test rather than hanging it.
async fn prepare(
    job: &std::sync::Arc<super::super::NativeCodingJob>,
    seed: Vec<(String, String)>,
) -> Result<(), CodingError> {
    let inputs = TurnInputs {
        seed,
        ..TurnInputs::default()
    };
    tokio::time::timeout(RUN, job.run_prepare(&NoopEventSink, inputs))
        .await
        .expect("the open must settle")
}

/// The messages one provider request carried.
fn messages(body: &str) -> Vec<Value> {
    let body: Value = serde_json::from_str(body).expect("the provider request is JSON");
    body["messages"]
        .as_array()
        .cloned()
        .expect("the request states its messages")
}

/// The history the request replayed: every message between the one system
/// message and the turn's own prompt, which is last.
fn history(messages: &[Value]) -> &[Value] {
    &messages[1..messages.len() - 1]
}

fn role(message: &Value) -> &str {
    message["role"].as_str().expect("a message states its role")
}

fn text(message: &Value) -> &str {
    message["content"]
        .as_str()
        .expect("a replayed message is text")
}

/// The bytes the shared assembly charges for one message: role plus content.
fn message_bytes(message: &Value) -> usize {
    role(message).len() + text(message).len()
}

/// Test 1 — the seed is history on the very first turn: two pairs arrive as
/// four messages, oldest first, before the turn's own message.
#[tokio::test]
async fn a_prepared_run_sends_its_seed_as_history_on_the_first_turn() {
    let provider = Scripted::start();
    let fixture = in_place("seed-first-turn", &provider.endpoint).await;
    let seeded = prepare(
        &fixture.job,
        vec![
            pair("SEED-OLDER-ASK".into(), "SEED-OLDER-ANSWER".into()),
            pair("SEED-NEWER-ASK".into(), "SEED-NEWER-ANSWER".into()),
        ],
    )
    .await;
    assert!(seeded.is_ok(), "the run must open with a seed: {seeded:?}");

    let first = next_turn(&fixture.job, "q").await;
    assert!(first.is_ok(), "the first message must run: {first:?}");

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        1,
        "the first message sends exactly one request: {bodies:?}"
    );
    let sent = messages(&bodies[0]);
    assert_eq!(
        sent.len(),
        6,
        "two seed pairs are four messages besides the system one and the turn's own: {}",
        bodies[0]
    );
    let replayed = history(&sent);
    let said: Vec<&str> = replayed.iter().map(text).collect();
    assert_eq!(
        replayed.iter().map(role).collect::<Vec<_>>(),
        vec!["user", "assistant", "user", "assistant"],
        "each seeded pair keeps chat's shape: {}",
        bodies[0]
    );
    assert_eq!(said[0], "SEED-OLDER-ASK", "the oldest pair comes first");
    assert_eq!(said[1], "SEED-OLDER-ANSWER");
    assert_eq!(said[2], "SEED-NEWER-ASK");
    assert_eq!(said[3], "SEED-NEWER-ANSWER");
    let asked = sent.last().expect("the turn's own prompt is last");
    assert_eq!(role(asked), "user");
    assert!(
        text(asked).ends_with('q'),
        "the turn's own message follows the seeded history: {}",
        text(asked)
    );
}

/// Test 2 — the seed is bounded by the run's context byte budget, by the same
/// rule completed turns follow: the newest pair survives, the oldest are
/// dropped.
#[tokio::test]
async fn a_seed_past_the_context_budget_loses_its_oldest_pairs_first() {
    let provider = Scripted::start();
    let fixture = in_place_with_budget("seed-bounded", &provider.endpoint, BUDGET).await;
    let seeded = prepare(
        &fixture.job,
        vec![
            pair(
                padded("MARKER-OLDEST-ASK", 9_000),
                padded("MARKER-OLDEST-ANSWER", 9_000),
            ),
            pair(
                padded("MARKER-MIDDLE-ASK", 400),
                padded("MARKER-MIDDLE-ANSWER", 400),
            ),
            pair(
                padded("MARKER-NEWEST-ASK", 400),
                padded("MARKER-NEWEST-ANSWER", 400),
            ),
        ],
    )
    .await;
    assert!(seeded.is_ok(), "the run must open with a seed: {seeded:?}");

    let first = next_turn(&fixture.job, "q").await;
    assert!(first.is_ok(), "the first message must run: {first:?}");

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        1,
        "the first message sends exactly one request: {bodies:?}"
    );
    assert!(
        !bodies[0].contains("MARKER-OLDEST"),
        "the oldest seeded pair is dropped first: {}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("MARKER-NEWEST"),
        "the newest seeded pair is always kept: {}",
        bodies[0]
    );
    let sent = messages(&bodies[0]);
    let replayed = history(&sent);
    let replayed_bytes: usize = replayed.iter().map(message_bytes).sum();
    let turn_bytes = message_bytes(&sent[0])
        + sent
            .last()
            .map(message_bytes)
            .expect("the turn's own prompt is present");
    assert!(
        replayed_bytes + turn_bytes <= BUDGET,
        "the seeded history stays inside the run's context byte budget: \
         {replayed_bytes} + {turn_bytes} > {BUDGET}"
    );
}

/// Test 4 — the seed is not lost after the first turn: turn two carries the
/// seed and turn one, in that order.
#[tokio::test]
async fn a_second_turn_carries_the_seed_and_the_first_turn() {
    let provider = Scripted::start();
    let fixture = in_place("seed-second-turn", &provider.endpoint).await;
    let seeded = prepare(
        &fixture.job,
        vec![
            pair("SEED-OLDER-ASK".into(), "SEED-OLDER-ANSWER".into()),
            pair("SEED-NEWER-ASK".into(), "SEED-NEWER-ANSWER".into()),
        ],
    )
    .await;
    assert!(seeded.is_ok(), "the run must open with a seed: {seeded:?}");

    assert!(
        next_turn(&fixture.job, "first message").await.is_ok(),
        "the first message must run"
    );
    assert!(
        next_turn(&fixture.job, "second message").await.is_ok(),
        "the second message must run"
    );

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        2,
        "each turn sends exactly one request: {bodies:?}"
    );
    let sent = messages(&bodies[1]);
    let replayed = history(&sent);
    assert_eq!(
        replayed.len(),
        6,
        "turn two replays the two seed pairs and turn one: {}",
        bodies[1]
    );
    let said: Vec<&str> = replayed.iter().map(text).collect();
    assert_eq!(said[0], "SEED-OLDER-ASK", "the seed still opens the run");
    assert_eq!(
        said[3], "SEED-NEWER-ANSWER",
        "and closes with its own answer"
    );
    assert!(
        said[4].contains("first message"),
        "turn one's prompt follows the seed: {}",
        said[4]
    );
    assert!(
        said[5].contains("turn 0 answered"),
        "and turn one's answer after it: {}",
        said[5]
    );
    let asked = sent.last().expect("the turn's own prompt is last");
    assert!(
        text(asked).contains("second message"),
        "the new message is the turn's own prompt, after the history: {}",
        text(asked)
    );
}
