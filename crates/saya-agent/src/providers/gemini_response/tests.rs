use super::*;
use serde_json::json;

#[test]
fn test_parse_text_response() {
    let body = json!({
        "candidates": [{
            "content": {
                "parts": [{"text": "Hello from Gemini!"}]
            }
        }]
    });

    let response = parse(body).unwrap();
    assert_eq!(response.message.role, "assistant");
    assert_eq!(response.message.content, "Hello from Gemini!");
    assert!(response.message.tool_calls.is_empty());
}

#[test]
fn test_parse_function_call_response() {
    let body = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "functionCall": {
                        "name": "weather",
                        "args": {"location": "Paris"}
                    }
                }]
            }
        }]
    });

    let response = parse(body).unwrap();
    assert_eq!(response.message.role, "assistant");
    assert!(response.message.content.is_empty());
    assert_eq!(response.message.tool_calls.len(), 1);
    assert_eq!(response.message.tool_calls[0].id, "gemini-weather");
    assert_eq!(response.message.tool_calls[0].name, "weather");
    assert_eq!(
        response.message.tool_calls[0].arguments,
        json!({"location": "Paris"})
    );
}

#[test]
fn test_parse_invalid_response() {
    assert!(parse(json!({})).is_err());
    assert!(parse(json!({"candidates": []})).is_err());
    assert!(parse(json!({"candidates": [{"content": {"parts": []}}]})).is_err());
}

/// Gemini's `complete()` bypasses
/// `collect()` and returns `gemini_response::parse(value)`, so the usage
/// must be threaded here — the `let _ = usage(&body);` discard is gone.
/// A `usageMetadata` carrying the cache-read and reasoning counts reaches
/// `response.usage`.
#[test]
fn parse_threads_usage_into_chat_response() {
    let body = json!({
        "candidates": [{"content": {"parts": [{"text": "hi"}]}}],
        "usageMetadata": {
            "promptTokenCount": 100,
            "candidatesTokenCount": 200,
            "cachedContentTokenCount": 90,
            "thoughtsTokenCount": 270
        }
    });
    let response = parse(body).expect("parses");
    let usage = response.usage.expect("usage reached the response");
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 470);
    assert_eq!(usage.cached_input_tokens, Some(90));
    assert_eq!(usage.reasoning_tokens, Some(270));
    assert_eq!(usage.cache_creation_input_tokens, None);
}

/// Gemini override path, absent case: a response with no
/// `usageMetadata` leaves `response.usage` `None`, distinct from a present
/// block reporting zeros — a silent Gemini response is not a free one.
#[test]
fn parse_leaves_usage_none_when_no_usage_metadata() {
    let body = json!({
        "candidates": [{"content": {"parts": [{"text": "hi"}]}}]
    });
    let response = parse(body).expect("parses");
    assert_eq!(response.usage, None);
}

/// Deliverable 4 (Gemini, with): `usageMetadata` carrying the cache-read
/// and reasoning counts populates `cached_input_tokens` and
/// `reasoning_tokens`. Note Gemini's `thoughtsTokenCount` is separate from
/// `candidatesTokenCount`, not inclusive of output.
#[test]
fn usage_metadata_populates_cache_and_reasoning() {
    let body = json!({
        "candidates": [{
            "content": {"parts": [{"text": "hi"}]}
        }],
        "usageMetadata": {
            "promptTokenCount": 100,
            "candidatesTokenCount": 200,
            "cachedContentTokenCount": 90,
            "thoughtsTokenCount": 270
        }
    });
    let usage = usage(&body).expect("present usageMetadata yields Some");
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 470);
    assert_eq!(usage.cached_input_tokens, Some(90));
    assert_eq!(usage.reasoning_tokens, Some(270));
    assert_eq!(usage.cache_creation_input_tokens, None);
}

/// Deliverable 4 (Gemini, without): a response with no `usageMetadata`
/// yields `None` (absent is not zero — a silent Gemini response is not
/// mistaken for one that reported zeros).
#[test]
fn missing_usage_metadata_is_none() {
    let body = json!({
        "candidates": [{
            "content": {"parts": [{"text": "hi"}]}
        }]
    });
    assert_eq!(usage(&body), None);
}

/// Deliverable 5 (Gemini): a reported `cachedContentTokenCount: 0` stays
/// `Some(0)` inside the returned usage, distinct from an absent field.
#[test]
fn reported_zero_cache_is_some_zero() {
    let body = json!({
        "usageMetadata": {"promptTokenCount": 10, "cachedContentTokenCount": 0}
    });
    let usage = usage(&body).expect("present usageMetadata yields Some");
    assert_eq!(usage.cached_input_tokens, Some(0));
}

/// parts marked `thought: true` carry
/// the chain-of-thought; it reaches `response.reasoning`, separate from
/// the answer's content. Gemini's `complete()` bypasses `collect()`, so
/// reasoning is parsed directly here.
#[test]
fn thought_parts_carry_reasoning_separate_from_content() {
    let body = json!({
        "candidates": [{
            "content": {
                "parts": [
                    {"text": "the time column looks nullable", "thought": true},
                    {"text": "use return_date"}
                ]
            }
        }]
    });
    let response = parse(body).expect("parses");
    assert_eq!(response.message.content, "use return_date");
    assert_eq!(
        response.reasoning.as_deref(),
        Some("the time column looks nullable"),
        "thought:true parts are reasoning, not content"
    );
}

/// a response whose parts carry no
/// `thought: true` flag leaves `response.reasoning` `None`, and content
/// parses normally — a non-reasoning response is unaffected.
#[test]
fn response_without_thought_parts_leaves_reasoning_none() {
    let body = json!({
        "candidates": [{
            "content": {"parts": [{"text": "just an answer"}]}
        }]
    });
    let response = parse(body).expect("parses");
    assert_eq!(response.message.content, "just an answer");
    assert_eq!(response.reasoning, None);
}
