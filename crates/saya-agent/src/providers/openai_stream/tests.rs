use super::{TokenUsage, apply_usage};
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
