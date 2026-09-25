//! Tests for the reasoning gate (`show_thinking`), driven through `super::apply_event` (moved verbatim
//! from the inline `tests` module in `stream_events/apply.rs`).

use super::{BlockKind, Transcript, apply_event};
use saya_agent::AgentEvent;

/// The model's chain-of-thought reaches the transcript only when the user
/// opted in. Off by default, so a user who did not ask for it never sees it;
/// on, it lands as a `Thinking` block separate from the assistant answer.
/// Asserts on the transcript state, the same seam the other `apply_event`
/// tests use.
#[test]
fn reasoning_text_is_silent_when_thinking_is_off() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::reasoning_text("I considered the time column"),
        false,
    );
    assert!(
        t.blocks().is_empty(),
        "reasoning must not reach the transcript when thinking is off: {:?}",
        t.blocks()
    );
    // It stays silent even when an answer has already streamed — it does
    // not push a block above, below, or between assistant blocks.
    apply_event(&mut t, AgentEvent::assistant_text("the answer"), false);
    apply_event(
        &mut t,
        AgentEvent::reasoning_text("more thinking mid-turn"),
        false,
    );
    assert_eq!(
        t.blocks().len(),
        1,
        "only the assistant block should be present: {:?}",
        t.blocks()
    );
    assert_eq!(t.blocks()[0].kind, BlockKind::Assistant);
    assert!(
        !t.blocks()[0].text.contains("thinking"),
        "reasoning must not be folded into the assistant block: {:?}",
        t.blocks()[0].text
    );
}

/// When thinking is on, a `ReasoningText` event pushes a `Thinking` block
/// carrying the chain-of-thought — separate from the assistant answer, so
/// it cannot be mistaken for it. Empty reasoning pushes nothing either way.
#[test]
fn reasoning_text_pushes_a_thinking_block_when_thinking_is_on() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::reasoning_text("I considered the time column"),
        true,
    );
    assert_eq!(t.blocks().len(), 1, "one thinking block is pushed");
    assert_eq!(t.blocks()[0].kind, BlockKind::Thinking);
    assert_eq!(t.blocks()[0].text, "I considered the time column");

    // Empty reasoning pushes nothing, even with thinking on.
    apply_event(&mut t, AgentEvent::reasoning_text(""), true);
    assert_eq!(t.blocks().len(), 1, "empty reasoning pushes no block");
}

/// Displaying the chain-of-thought must not open a path for it to reach a
/// persisted session. The reasoning restates row values and column
/// contents in prose, and session files are redacted against a different
/// threat, so the display toggle and the persistence boundary stay
/// independent: turning the former on must not weaken the latter.
///
/// What this pins is the display half: the reasoning really reaches the
/// transcript as its own block. The session built alongside it is a
/// separate object, so the absence checks below are a shape check on the
/// persisted form, not proof that a live turn cannot carry reasoning into
/// it — that guarantee is structural and pinned elsewhere, by
/// `record_turn` taking no reasoning argument and by `ChatMessage`'s wire
/// form being fixed by test.
#[test]
fn reasoning_shown_on_screen_stays_out_of_the_persisted_session() {
    use crate::interactive::session_state::SessionState;

    let reasoning = "the secret chain-of-thought about row values 9f3a";
    let mut transcript = Transcript::default();
    apply_event(&mut transcript, AgentEvent::reasoning_text(reasoning), true);
    apply_event(
        &mut transcript,
        AgentEvent::assistant_text("the answer is 42"),
        true,
    );

    assert!(
        transcript
            .blocks()
            .iter()
            .any(|b| b.kind == BlockKind::Thinking && b.text.contains(reasoning)),
        "the reasoning must be on screen for this test to prove anything"
    );

    let mut session = SessionState::new("s1", Some(String::from("analytics")), String::from("m"));
    session.show_thinking = true;
    session.record_turn("what is the answer", "the answer is 42", false, Vec::new());

    let json = serde_json::to_string(&session).expect("serializes");
    assert!(
        !json.contains(reasoning),
        "reasoning on screen leaked into the persisted session: {json}"
    );

    let replayed = session.provider_history();
    assert!(
        replayed.iter().all(|m| !m.content.contains(reasoning)),
        "reasoning on screen leaked into replayed history: {replayed:?}"
    );

    let redacted = session.redacted();
    let redacted_json = serde_json::to_string(&redacted).expect("serializes");
    assert!(
        !redacted_json.contains(reasoning),
        "reasoning on screen leaked into the redacted session: {redacted_json}"
    );
}
