//! A further root turn carries the earlier turns' conversation, bounded like
//! chat (phase-3 unit 3-B).
//!
//! Chat builds each turn's history from the (prompt, final answer) pairs of
//! the turns before it and leaves the bounding to the one shared assembly.
//! These drive the same reading at the provider: the earlier pairs arrive as
//! history messages before the turn's own prompt, a later turn's history
//! begins exactly where the one before it did, and past the context byte
//! budget the oldest turns are the ones dropped.

use super::super::parent_turn_fixture::{in_place, in_place_with_budget};
use super::super::parent_turn_provider::Scripted;
use super::{first_turn, next_turn};
use serde_json::Value;

/// The run's context byte budget the bounded case states: the value chat's
/// own history is bounded by, small enough that a few large turns cannot fit.
const BUDGET: usize = 12 * 1024;

/// One completed turn's prompt, long enough that only the newest pair fits
/// the budget above: system plus this turn's own message leaves room for one
/// such pair and not two. The marker makes the turn identifiable in the wire
/// body.
fn padded(marker: &str) -> String {
    format!("{marker} {}", "x".repeat(6_000 - marker.len() - 1))
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

#[tokio::test]
async fn a_second_turn_replays_the_first_turns_prompt_and_answer() {
    let provider = Scripted::start();
    let fixture = in_place("history-second", &provider.endpoint).await;
    first_turn(&fixture.job).await.expect("the first turn runs");
    next_turn(&fixture.job, "second message")
        .await
        .expect("the second turn runs");

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        2,
        "each turn sends exactly one request: {bodies:?}"
    );
    let second = messages(&bodies[1]);
    let replayed = history(&second);
    assert_eq!(
        replayed.len(),
        2,
        "turn two replays turn one as one pair: {}",
        bodies[1]
    );
    assert_eq!(
        role(&replayed[0]),
        "user",
        "the replayed pair opens with the earlier prompt: {}",
        bodies[1]
    );
    assert!(
        text(&replayed[0]).contains("first message: read this folder"),
        "turn one's prompt is replayed: {}",
        text(&replayed[0])
    );
    assert_eq!(
        role(&replayed[1]),
        "assistant",
        "and closes with its answer: {}",
        bodies[1]
    );
    assert!(
        text(&replayed[1]).contains("turn 0 answered"),
        "turn one's answer is replayed: {}",
        text(&replayed[1])
    );
    let asked = second.last().expect("the turn's own prompt is last");
    assert_eq!(role(asked), "user");
    assert!(
        text(asked).contains("second message"),
        "the new message is the turn's own prompt, after the history: {}",
        text(asked)
    );
}

#[tokio::test]
async fn a_third_turns_history_starts_exactly_where_the_seconds_left_off() {
    let provider = Scripted::start();
    let fixture = in_place("history-prefix", &provider.endpoint).await;
    first_turn(&fixture.job).await.expect("the first turn runs");
    next_turn(&fixture.job, "second message")
        .await
        .expect("the second turn runs");
    next_turn(&fixture.job, "third message")
        .await
        .expect("the third turn runs");

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        3,
        "each turn sends exactly one request: {bodies:?}"
    );
    let second = messages(&bodies[1]);
    let third = messages(&bodies[2]);
    let (two, three) = (history(&second), history(&third));
    assert_eq!(two.len(), 2, "turn two replays one pair: {}", bodies[1]);
    assert_eq!(
        three.len(),
        4,
        "turn three replays two pairs: {}",
        bodies[2]
    );
    let prefix = serde_json::to_string(&three[..two.len()]).expect("history serializes");
    assert_eq!(
        prefix,
        serde_json::to_string(two).expect("history serializes"),
        "turn three's history must begin byte-identically with turn two's, so the \
         provider's prompt cache keeps what it already has"
    );
}

#[tokio::test]
async fn a_long_conversation_is_bounded_and_loses_its_oldest_turns_first() {
    let provider = Scripted::start();
    let fixture = in_place_with_budget("history-bounded", &provider.endpoint, BUDGET).await;
    first_turn(&fixture.job).await.expect("the first turn runs");
    next_turn(&fixture.job, &padded("MARKER-OLDEST"))
        .await
        .expect("the second turn runs");
    next_turn(&fixture.job, &padded("MARKER-NEWEST"))
        .await
        .expect("the third turn runs");
    next_turn(&fixture.job, "one more")
        .await
        .expect("the fourth turn runs");

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        4,
        "each turn sends exactly one request: {bodies:?}"
    );
    let fourth = messages(&bodies[3]);
    let replayed = history(&fourth);
    assert!(
        replayed
            .iter()
            .any(|said| text(said).contains("MARKER-NEWEST")),
        "the newest completed turn survives the bound: {}",
        bodies[3]
    );
    assert!(
        !bodies[3].contains("MARKER-OLDEST"),
        "the oldest turns are dropped first: {}",
        bodies[3]
    );
    assert!(
        !bodies[3].contains("first message: read this folder"),
        "the run's own first turn is dropped too: {}",
        bodies[3]
    );
    let turn_bytes = message_bytes(&fourth[0])
        + fourth
            .last()
            .map(message_bytes)
            .expect("the turn's own prompt is present");
    let replayed_bytes: usize = replayed.iter().map(message_bytes).sum();
    assert!(
        replayed_bytes + turn_bytes <= BUDGET,
        "the replayed history stays inside the context byte budget chat uses: \
         {replayed_bytes} + {turn_bytes} > {BUDGET}"
    );
}

#[tokio::test]
async fn the_first_turn_carries_no_history() {
    let provider = Scripted::start();
    let fixture = in_place("history-first", &provider.endpoint).await;
    first_turn(&fixture.job).await.expect("the first turn runs");

    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        1,
        "the first turn sends exactly one request: {bodies:?}"
    );
    let first = messages(&bodies[0]);
    assert_eq!(
        first.len(),
        2,
        "the first turn is the system prompt and its own message, nothing replayed: {}",
        bodies[0]
    );
    assert_eq!(role(&first[0]), "system");
    assert_eq!(role(&first[1]), "user");
    assert!(
        text(&first[1]).contains("first message: read this folder"),
        "the turn's own prompt is the one user message: {}",
        text(&first[1])
    );
}
