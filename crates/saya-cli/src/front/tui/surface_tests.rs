//! What `/clear`, `/compact` and a state change do to the **live** run: the
//! next message after one carries what the command stated, not what the run was
//! opened with.
//!
//! Every test drives the surface's own port — the entry the loop's commands are
//! performed on — over a real run against a scripted provider, and reads the
//! request bodies the provider actually received.

#![cfg(unix)]

use super::run_cmd::CmdPort;
use super::surface_fixture::surface;

/// The phrase the compaction's own request carries: the engine's prompt.
const SUMMARISE: &str = "Summarise the following earlier conversation turns";

/// Two turns, `/clear`, a third turn: the third request carries no earlier turn.
#[tokio::test]
async fn a_clear_drops_the_turns_the_live_run_was_opened_with() {
    let fixture = surface("clear").await;
    fixture.port.send("first message".to_owned());
    fixture.settle(1).await;
    fixture.port.send("second message".to_owned());
    fixture.settle(2).await;

    fixture.op("/clear").await;

    fixture.port.send("third message".to_owned());
    fixture.settle(3).await;
    let bodies = fixture.bodies();
    let third = bodies.last().expect("three requests arrived");
    assert!(
        third.contains("third message"),
        "the third request is the third turn's: {third}"
    );
    assert!(
        !third.contains("first message"),
        "the cleared run replays nothing: {third}"
    );
    assert!(
        !third.contains("second message"),
        "the cleared run replays nothing: {third}"
    );
}

/// `/compact` then a turn: the request carries the summary the compaction
/// saved, not the raw turns it folded.
#[tokio::test]
async fn a_compact_puts_its_summary_in_the_next_request() {
    let fixture = surface("compact").await;
    for (at, prompt) in [
        "first message",
        "second message",
        "third message",
        "fourth message",
    ]
    .iter()
    .enumerate()
    {
        fixture.port.send((*prompt).to_owned());
        fixture.settle(at + 1).await;
    }

    fixture.op("/compact").await;
    fixture.port.send("fifth message".to_owned());
    // The summarising request comes first, then the turn after it.
    fixture.settle(6).await;

    let bodies = fixture.bodies();
    let summarised = bodies
        .iter()
        .position(|body| body.contains(SUMMARISE))
        .expect("the compaction asked the provider for a summary");
    let summary = format!("turn {summarised} answered");
    let last = bodies
        .last()
        .expect("the turn after the compaction arrived");
    assert!(
        last.contains(&summary),
        "the next request carries the saved summary ({summary}): {last}"
    );
    assert!(
        last.contains("fifth message"),
        "and the turn the operator just sent: {last}"
    );
    assert!(
        !last.contains("first message"),
        "the turns the summary replaced are not replayed: {last}"
    );
}

/// `/model X` before the run has taken a turn: the next message is composed
/// from the state the change stated, exactly as the line front's first message
/// is composed after a slash command gave its prepared run up.
#[tokio::test]
async fn a_model_change_before_the_first_turn_names_the_new_model() {
    let fixture = surface("model").await;
    fixture.op("/model chosen-model").await;

    fixture.port.send("which model".to_owned());
    fixture.settle(1).await;
    let body = fixture
        .bodies()
        .pop()
        .expect("the first turn's request arrived");
    assert!(
        body.contains("\"model\":\"chosen-model\"") || body.contains("\"model\": \"chosen-model\""),
        "the request names the model the session states: {body}"
    );
    assert_eq!(
        fixture.state().model,
        "chosen-model",
        "the session's own fact moved with it"
    );
}
