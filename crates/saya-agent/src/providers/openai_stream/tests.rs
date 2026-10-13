use super::{State, TokenUsage, apply_usage};
use crate::ProviderEvent;
use serde_json::json;

/// Deliverable 4 (OpenAI mapping, with): a usage chunk with detail objects
/// populates `cached_input_tokens` and `reasoning_tokens` on the running
/// total.
#[test]
fn apply_usage_populates_cache_and_reasoning() {
    let mut accumulated = TokenUsage::default();
    let usage: super::super::openai_chunks::Usage = serde_json::from_value(json!({
        "prompt_tokens": 100,
        "completion_tokens": 200,
        "prompt_tokens_details": {"cached_tokens": 90},
        "completion_tokens_details": {"reasoning_tokens": 270}
    }))
    .expect("parses");
    apply_usage(&mut accumulated, &usage);
    assert_eq!(accumulated.input_tokens, 100);
    assert_eq!(accumulated.output_tokens, 200);
    assert_eq!(accumulated.cached_input_tokens, Some(90));
    assert_eq!(accumulated.reasoning_tokens, Some(270));
    assert_eq!(accumulated.cache_creation_input_tokens, None);
}

/// Deliverable 4 (OpenAI mapping, without): a usage chunk with no detail
/// objects leaves the new fields `None` and does not clobber a previously
/// reported cache hit — the trailing chunk for one response carries the
/// final numbers, but detail absence is an honest `None`, not a reset.
#[test]
fn apply_usage_without_details_leaves_new_fields_none() {
    let mut accumulated = TokenUsage::default();
    let usage: super::super::openai_chunks::Usage =
        serde_json::from_value(json!({"prompt_tokens": 5, "completion_tokens": 6}))
            .expect("parses");
    apply_usage(&mut accumulated, &usage);
    assert_eq!(accumulated.input_tokens, 5);
    assert_eq!(accumulated.output_tokens, 6);
    assert_eq!(accumulated.cached_input_tokens, None);
    assert_eq!(accumulated.reasoning_tokens, None);
}

/// Deliverable 5 (OpenAI mapping): a reported `cached_tokens: 0` flows
/// through as `Some(0)`, never collapsed to `None`.
#[test]
fn apply_usage_reports_zero_cache_as_some_zero() {
    let mut accumulated = TokenUsage::default();
    let usage: super::super::openai_chunks::Usage = serde_json::from_value(json!({
        "prompt_tokens": 10,
        "prompt_tokens_details": {"cached_tokens": 0}
    }))
    .expect("parses");
    apply_usage(&mut accumulated, &usage);
    assert_eq!(accumulated.cached_input_tokens, Some(0));
}

/// Pushes one usage-only SSE frame (empty `choices`) carrying the given usage.
fn push_usage_chunk(state: &mut State, usage: serde_json::Value) {
    let frame = format!("data: {}\n\n", json!({"choices": [], "usage": usage}));
    state.push(frame.as_bytes()).expect("chunk parses");
}

fn events(state: &mut State) -> Vec<ProviderEvent> {
    state.pending.drain(..).collect()
}

/// A final chunk with only `completion_tokens` emits no `Usage`: an unreported
/// prompt side is unknown, never a measured zero.
#[test]
fn usage_chunk_with_only_completion_stays_unreported() {
    let mut state = State::default();
    push_usage_chunk(&mut state, json!({"completion_tokens": 7}));
    assert!(state.pending.is_empty(), "no Usage without prompt side");
    assert!(!state.usage_reported);
}

/// A final chunk with both totals emits exactly one `Usage` carrying both.
#[test]
fn usage_chunk_with_both_totals_emits_one_usage() {
    let mut state = State::default();
    push_usage_chunk(
        &mut state,
        json!({"prompt_tokens": 5, "completion_tokens": 7}),
    );
    assert_eq!(
        events(&mut state),
        vec![ProviderEvent::Usage(TokenUsage::new(5, 7))]
    );
    assert!(state.usage_reported);
}

/// Totals split across chunks emit `Usage` once, after both sides are seen.
#[test]
fn usage_split_across_chunks_emits_once_both_seen() {
    let mut state = State::default();
    push_usage_chunk(&mut state, json!({"prompt_tokens": 5}));
    assert!(state.pending.is_empty(), "input alone stays unreported");
    push_usage_chunk(&mut state, json!({"completion_tokens": 7}));
    assert_eq!(
        events(&mut state),
        vec![ProviderEvent::Usage(TokenUsage::new(5, 7))]
    );
    assert!(state.usage_reported);
}

/// A chunk reporting `0` on both totals emits `Usage` with zeros: a reported
/// zero is measured.
#[test]
fn usage_chunk_with_reported_zeros_emits_usage() {
    let mut state = State::default();
    push_usage_chunk(
        &mut state,
        json!({"prompt_tokens": 0, "completion_tokens": 0}),
    );
    assert_eq!(
        events(&mut state),
        vec![ProviderEvent::Usage(TokenUsage::new(0, 0))]
    );
    assert!(state.usage_reported);
}
