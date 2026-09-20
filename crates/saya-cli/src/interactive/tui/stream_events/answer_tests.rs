//! Tests for the assistant-answer lifecycle (deltas, resets, completion), driven through `super::apply_event` (moved verbatim
//! from the inline `tests` module in `stream_events/apply.rs`).

use super::{BlockKind, Transcript, apply_event};
use saya_agent::AgentEvent;

fn last_block_text(transcript: &Transcript) -> Option<&str> {
    transcript.blocks().last().map(|b| b.text.as_str())
}

/// A `TurnReset` (mid-stream failure, turn retrying) clears the assistant
/// text accumulated so far, so the re-streamed answer **replaces** it —
/// the transcript must never show the partial attempt concatenated with
/// the full retry.
#[test]
fn turn_reset_replaces_the_accumulated_answer_rather_than_appending() {
    let mut t = Transcript::new();
    apply_event(&mut t, AgentEvent::assistant_text("The an"), false);
    apply_event(&mut t, AgentEvent::assistant_text("sw"), false);
    assert_eq!(last_block_text(&t), Some("The answ"));

    apply_event(&mut t, AgentEvent::turn_reset(), false);
    apply_event(
        &mut t,
        AgentEvent::assistant_text("The answer is 42."),
        false,
    );
    assert_eq!(
        last_block_text(&t),
        Some("The answer is 42."),
        "the retried answer must replace the partial text, not append to it"
    );
    // The re-streamed answer stays a single assistant block.
    assert_eq!(
        t.blocks()
            .iter()
            .filter(|b| b.kind == BlockKind::Assistant)
            .count(),
        1
    );
}

/// A reset with nothing streamed yet is a no-op: there is nothing to
/// discard, and the next answer still opens its own block.
#[test]
fn turn_reset_with_nothing_streamed_is_a_no_op() {
    let mut t = Transcript::new();
    t.push(BlockKind::System, "memory off · recall disabled");
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert_eq!(t.blocks().len(), 1, "nothing streamed → nothing to clear");
    apply_event(&mut t, AgentEvent::assistant_text("the answer"), false);
    assert_eq!(last_block_text(&t), Some("the answer"));
}
