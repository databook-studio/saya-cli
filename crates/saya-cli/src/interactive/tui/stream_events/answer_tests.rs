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
    // The notice trails the discarded attempt; the retry resumes the
    // emptied assistant block behind it, so the tail reads
    // `[answer][notice]` and the answer still replaces (never
    // concatenates) the partial.
    apply_event(
        &mut t,
        AgentEvent::assistant_text("The answer is 42."),
        false,
    );
    let assistant: Vec<_> = t
        .blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::Assistant)
        .collect();
    assert_eq!(
        assistant.len(),
        1,
        "the retried answer stays one assistant block: {:?}",
        t.blocks()
    );
    assert_eq!(
        assistant[0].text,
        "The answer is 42.",
        "the retried answer must replace the partial text, not append to it: {:?}",
        t.blocks()
    );
    assert_eq!(
        last_block_text(&t),
        Some(piped_retry_notice().as_str()),
        "the retry notice trails the resumed answer: {:?}",
        t.blocks()
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

/// The piped surface's wording for a turn retry, derived from the same
/// `TerminalEvent::TurnReset` adapter the pipe renders — so the two
/// surfaces cannot drift. Trimmed: transcript blocks carry no trailing
/// newline.
fn piped_retry_notice() -> String {
    use crate::render::{RenderFormat, TerminalEvent, render_event};
    let rendered = render_event(&TerminalEvent::TurnReset, RenderFormat::Text);
    rendered.stdout.trim_end_matches('\n').to_string()
}

/// When a turn is retried, the TUI says so: a half-written answer must
/// not silently vanish and start over with no explanation.
#[test]
fn a_retry_is_announced_in_the_transcript() {
    let mut t = Transcript::new();
    apply_event(&mut t, AgentEvent::assistant_text("The an"), false);
    apply_event(&mut t, AgentEvent::assistant_text("sw"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        t.blocks().iter().any(|b| b.text == piped_retry_notice()),
        "a retry must leave a visible notice in the transcript: {:?}",
        t.blocks()
    );
}

/// The TUI's retry notice is exactly the piped surface's wording: the
/// test that stops the two surfaces drifting again.
#[test]
fn the_retry_notice_matches_the_piped_surface() {
    let mut t = Transcript::new();
    apply_event(&mut t, AgentEvent::assistant_text("The an"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    let notice = t
        .blocks()
        .iter()
        .find(|b| b.text == piped_retry_notice())
        .expect("the retry notice must be present");
    assert_eq!(notice.text, piped_retry_notice());
}

/// A recovering retry is not a failure: the notice renders as a `System`
/// block, never an `Error`.
#[test]
fn the_notice_is_a_system_block_not_an_error() {
    let mut t = Transcript::new();
    apply_event(&mut t, AgentEvent::assistant_text("The an"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    let notice = t
        .blocks()
        .iter()
        .find(|b| b.text == piped_retry_notice())
        .expect("the retry notice must be present");
    assert_eq!(
        notice.kind,
        BlockKind::System,
        "a recovering retry is not a failure: {:?}",
        t.blocks()
    );
    assert!(
        t.blocks().iter().all(|b| b.kind != BlockKind::Error),
        "the retry must not render as an error: {:?}",
        t.blocks()
    );
}

/// After the retry, the re-streamed answer resumes its own assistant block
/// *behind* the notice — it must not append to the notice. The retry
/// notice trails the resumed answer: the tail reads `[answer][notice]`.
#[test]
fn the_restreamed_answer_does_not_append_to_the_notice() {
    let mut t = Transcript::new();
    apply_event(&mut t, AgentEvent::assistant_text("The an"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    apply_event(
        &mut t,
        AgentEvent::assistant_text("The answer is 42."),
        false,
    );
    let blocks = t.blocks();
    let last = blocks.last().expect("the retry notice trails the answer");
    assert_eq!(last.kind, BlockKind::System);
    assert_eq!(last.text, piped_retry_notice());
    let answer = &blocks[blocks.len() - 2];
    assert_eq!(answer.kind, BlockKind::Assistant);
    assert_eq!(answer.text, "The answer is 42.");
}

/// `TurnReset` discards by design: announcing the retry must not
/// resurrect the partial text streamed before the reset.
#[test]
fn the_discarded_partial_is_not_resurrected() {
    let mut t = Transcript::new();
    apply_event(&mut t, AgentEvent::assistant_text("The an"), false);
    apply_event(&mut t, AgentEvent::assistant_text("sw"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        t.blocks().iter().all(|b| !b.text.contains("The an")),
        "the discarded partial must be gone: {:?}",
        t.blocks()
    );
}

/// A turn with no retry says nothing extra: the packet must not announce
/// retries that did not happen.
#[test]
fn a_turn_with_no_retry_says_nothing_extra() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::assistant_text("The answer is 42."),
        false,
    );
    assert!(
        t.blocks().iter().all(|b| b.text != piped_retry_notice()),
        "no retry happened, so no notice may appear: {:?}",
        t.blocks()
    );
}

/// Each retry is a real event worth seeing: three retries leave three
/// notices, not one collapsed line. No counter — the transcript shows
/// the attempts that happened, never a total invented up front.
#[test]
fn repeated_retries_leave_one_notice_each() {
    let mut t = Transcript::new();
    apply_event(&mut t, AgentEvent::assistant_text("The an"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    apply_event(&mut t, AgentEvent::assistant_text("The ans"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    apply_event(
        &mut t,
        AgentEvent::assistant_text("The answer is 42."),
        false,
    );
    let notices = t
        .blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::System && b.text == piped_retry_notice())
        .count();
    assert_eq!(
        notices,
        2,
        "each discarded attempt leaves its own notice: {:?}",
        t.blocks()
    );
}
