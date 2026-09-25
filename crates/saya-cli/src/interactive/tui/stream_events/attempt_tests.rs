//! Attempt-boundary tests for the answer lifecycle (re-audit R02).
//!
//! `TurnReset` means "discard what *this attempt* produced". Chapter scoping
//! (slice B) fixed crossing a `User` boundary; it does not fix crossing a
//! provider receive *within* one request, where an earlier step's preamble
//! and its completed tool results live in the same chapter. These replay the
//! re-audit's sequence against the transcript directly — no provider.

use super::answer::RETRY_NOTICE;
use super::{BlockKind, Transcript, apply_event};
use saya_agent::AgentEvent;

fn texts(t: &Transcript, kind: BlockKind) -> Vec<String> {
    t.blocks()
        .iter()
        .filter(|b| b.kind == kind)
        .map(|b| b.text.clone())
        .collect()
}

fn has_text(t: &Transcript, needle: &str) -> bool {
    t.blocks().iter().any(|b| b.text.contains(needle))
}

/// One request: a preamble, a tool run that completed, then a second receive
/// that dies before emitting any text. Everything delivered by the first step
/// must survive — it is evidence the user already has.
fn delivered_first_step() -> Transcript {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "do the thing");
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(
        &mut t,
        AgentEvent::assistant_text("let me look that up"),
        false,
    );
    apply_event(
        &mut t,
        AgentEvent::tool_requested(
            "bounded_sql_query",
            serde_json::json!({"sql": "select 1"}),
            None,
        ),
        false,
    );
    apply_event(
        &mut t,
        AgentEvent::ToolCompleted {
            name: "bounded_sql_query".into(),
            summary: "1 row".into(),
        },
        false,
    );
    t
}

#[test]
fn a_failed_receive_preserves_a_completed_tool_result() {
    let mut t = delivered_first_step();
    // The next receive begins and dies before any text.
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        has_text(&t, "1 row"),
        "a completed tool result from an earlier step must survive a later \
         receive failing:\n{:#?}",
        t.blocks()
    );
}

#[test]
fn a_failed_receive_preserves_an_earlier_preamble() {
    let mut t = delivered_first_step();
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert_eq!(
        texts(&t, BlockKind::Assistant),
        vec!["let me look that up".to_string()],
        "the delivered preamble must survive untouched"
    );
}

#[test]
fn a_failed_receive_that_streamed_nothing_says_nothing() {
    let mut t = delivered_first_step();
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        !has_text(&t, RETRY_NOTICE),
        "no text was streamed by the failed attempt, so there is no \
         discarded attempt to announce"
    );
}

#[test]
fn a_retry_after_partial_text_discards_only_that_text() {
    let mut t = delivered_first_step();
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(
        &mut t,
        AgentEvent::assistant_text(" and the answer is"),
        false,
    );
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        !has_text(&t, "and the answer is"),
        "the failed attempt's partial text goes"
    );
    assert!(
        has_text(&t, "let me look that up") && has_text(&t, "1 row"),
        "everything delivered before the attempt stays:\n{:#?}",
        t.blocks()
    );
    assert!(
        has_text(&t, RETRY_NOTICE),
        "text was discarded, so the retry is announced"
    );
}

#[test]
fn repeated_retries_roll_back_to_their_own_attempt() {
    let mut t = delivered_first_step();
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(&mut t, AgentEvent::assistant_text(" first try"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(&mut t, AgentEvent::assistant_text(" second try"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        !has_text(&t, "first try") && !has_text(&t, "second try"),
        "each attempt's own text is discarded by its own reset"
    );
    assert!(
        has_text(&t, "let me look that up") && has_text(&t, "1 row"),
        "the delivered first step survives both:\n{:#?}",
        t.blocks()
    );
}

/// Slice B's invariant, re-asserted here because this slice replaces the
/// path that enforced it.
#[test]
fn a_retry_still_does_not_touch_a_previous_chapter() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "old question");
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(&mut t, AgentEvent::assistant_text("old answer"), false);
    t.push(BlockKind::User, "new question");
    apply_event(&mut t, AgentEvent::turn_started(), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        has_text(&t, "old answer"),
        "the previous chapter's delivered answer must survive:\n{:#?}",
        t.blocks()
    );
}
