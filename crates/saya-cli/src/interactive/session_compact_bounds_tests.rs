//! Compaction summary bounds: an over-cap summary cut inside a multi-byte
//! character must fail validation or keep a char-boundary-safe summary —
//! never panic; a validated maximum summary must render whole at a large
//! context budget; a tiny budget must truncate flagged and char-safe; and a
//! vanished TUI worker must latch the same automatic-failure flag a failed
//! automatic compaction sets.

use super::compact_task;
use super::session_compact::{COMPACT_SUMMARY_MAX_CHARS, plan};
use super::session_compact_call::summarise;
use super::session_continuation::render_summary_block;
use super::session_state::SessionState;
use super::tui::types::app::idle_app;
use async_trait::async_trait;
use saya_agent::{ChatMessage, ChatProvider, ChatRequest, ChatResponse, ProviderError};

fn pinned_turns(count: usize) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..count {
        state.record_turn(
            format!("write chunk {index}"),
            format!("size: 10{index}"),
            false,
            Vec::new(),
        );
    }
    state
}

/// An over-cap summary whose byte 4096 lands inside an em dash: the cut stays
/// on a char boundary at or below the cap and never panics.
#[tokio::test]
async fn an_over_cap_summary_cut_inside_a_multibyte_char_stays_boundary_safe() {
    let state = turns(5);
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    let mut text = String::new();
    text.push_str(&"x".repeat(COMPACT_SUMMARY_MAX_CHARS - 1));
    text.push('—');
    assert_eq!(text.len(), COMPACT_SUMMARY_MAX_CHARS + 2);
    let outcome = summarise(&StaticProvider { text, usage: None }, "m", &plan)
        .await
        .expect("a boundary-safe cut keeps a summary");
    assert!(outcome.summary.len() <= COMPACT_SUMMARY_MAX_CHARS);
    assert!(outcome.summary.is_char_boundary(outcome.summary.len()));
}

/// A cut that drops a pinned token fails validation instead of keeping a
/// half-summary — and never panics even when the cut lands mid-character.
#[tokio::test]
async fn a_cut_that_drops_a_pinned_token_fails_validation() {
    let state = pinned_turns(5);
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    assert!(plan.pinned.len() >= 3, "the test needs pinned tokens");
    let mut text = String::new();
    for token in plan.pinned.iter().take(2) {
        text.push_str(token);
        text.push(' ');
    }
    text.push_str(&"x".repeat((COMPACT_SUMMARY_MAX_CHARS - 1) - text.len()));
    text.push('—');
    assert_eq!(text.len(), COMPACT_SUMMARY_MAX_CHARS + 2);
    let error = summarise(&StaticProvider { text, usage: None }, "m", &plan)
        .await
        .expect_err("the lost pin must fail the compaction");
    assert!(
        error.contains("pinned"),
        "the reason names the pin: {error}"
    );
}

/// A multibyte flood over the cap with the pinned tokens at the start: Ok,
/// whole characters only, at or below the cap.
#[tokio::test]
async fn an_over_cap_multibyte_summary_cuts_whole_characters() {
    let state = pinned_turns(5);
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    let mut text = String::new();
    for token in &plan.pinned {
        text.push_str(token);
        text.push(' ');
    }
    text.push('X');
    text.push_str(&"é".repeat(2135));
    assert!(
        text.len() >= 4300,
        "the flood overfills the cap: {}",
        text.len()
    );
    let outcome = summarise(&StaticProvider { text, usage: None }, "m", &plan)
        .await
        .expect("the leading pins survive the cut");
    assert!(outcome.summary.len() <= COMPACT_SUMMARY_MAX_CHARS);
    assert!(outcome.summary.contains("size: 100"), "the pin survives");
}

/// A validated maximum-size summary ending in a pinned-looking line renders
/// whole at a large context budget: not flagged, tail intact.
#[test]
fn a_maximum_summary_renders_whole_at_a_large_context_budget() {
    let mut text = String::new();
    text.push_str(&"x".repeat(COMPACT_SUMMARY_MAX_CHARS - 14));
    text.push_str("digest: abc123");
    assert_eq!(text.len(), COMPACT_SUMMARY_MAX_CHARS);
    let block = render_summary_block(&text, 100_000).expect("a summary renders");
    assert!(!block.truncated, "a maximum summary renders whole");
    assert!(
        block.body.ends_with("digest: abc123"),
        "the tail survives: {}",
        block.body
    );
}

/// An explicit tiny budget truncates flagged and stays char-safe: the cut is
/// on a char boundary and the body honours the budget.
#[test]
fn a_tiny_budget_truncates_flagged_and_char_safe() {
    let text = "—".repeat(1500);
    let block = render_summary_block(&text, 1024).expect("a summary renders");
    assert!(block.truncated, "a tiny budget truncates flagged");
    assert!(
        block.body.len() <= 1024,
        "the body honours the budget: {}",
        block.body.len()
    );
    assert!(block.body.is_char_boundary(block.body.len()));
}

/// A TUI worker that died without sending a result latches the same
/// automatic-failure flag a failed automatic compaction sets.
#[test]
fn a_vanished_worker_latches_the_automatic_failure_flag() {
    let mut app = idle_app();
    let mut state = turns(5);
    let (tx, rx) = std::sync::mpsc::channel();
    app.compact_task = Some(rx);
    drop(tx);
    compact_task::poll(&mut app, &mut state);
    assert!(app.compact_task.is_none(), "a polled worker is cleared");
    assert!(
        state.auto_compact_failed,
        "a vanished worker latches the no-retry flag"
    );
}
fn turns(count: usize) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..count {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}

struct StaticProvider {
    text: String,
    usage: Option<saya_agent::TokenUsage>,
}

#[async_trait]
impl ChatProvider for StaticProvider {
    fn name(&self) -> &str {
        "static"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut response = ChatResponse::new(ChatMessage::text("assistant", &self.text));
        response.usage = self.usage;
        Ok(response)
    }
}
