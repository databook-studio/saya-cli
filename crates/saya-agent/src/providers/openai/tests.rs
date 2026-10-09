use super::*;
use crate::{ChatMessage, LocalStateEffect, ToolDefinition, ToolEffect};

fn request_with(format: ResponseFormat, effort: ReasoningEffort) -> ChatRequest {
    ChatRequest {
        model: "test-model".into(),
        messages: vec![ChatMessage::text("user", "extract")],
        tools: vec![ToolDefinition {
            name: "schema_discovery".into(),
            description: "schema".into(),
            read_only: true,
            parameters: serde_json::json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: crate::ToolConcurrency::Serial,
            completion: None,
        }],
        response_format: format,
        reasoning_effort: effort,
    }
}

/// A JSON-mode request carries `response_format: {"type":"json_object"}`
/// on the OpenAI wire — the spelling the spec verified at 1.7s / 0 reasoning
/// tokens against the live gateway.
#[test]
fn json_object_request_carries_response_format_on_wire() {
    let body = OpenAiRequest::from_request(
        request_with(ResponseFormat::JsonObject, ReasoningEffort::Default),
        0.1,
        4096,
    );
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        json.contains(r#""response_format":{"type":"json_object"}"#),
        "json_object must appear on the wire: {json}"
    );
}

/// The configured output ceiling rides the OpenAI body as
/// `max_completion_tokens` (`max_tokens` is deprecated and rejected by
/// newer reasoning models). The default path sends the settings default,
/// so the ceiling is always explicit — never an unknown server default.
#[test]
fn configured_output_ceiling_rides_the_body_as_max_completion_tokens() {
    let body = OpenAiRequest::from_request(
        request_with(ResponseFormat::Text, ReasoningEffort::Default),
        0.1,
        1234,
    );
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        json.contains(r#""max_completion_tokens":1234"#),
        "configured ceiling must ride the body: {json}"
    );
    assert!(
        !json.contains("max_tokens\":") || json.contains("max_completion_tokens"),
        "no bare max_tokens spelling: {json}"
    );
}

/// A `Text` (default) request omits `response_format`
/// entirely, so the prose path is byte-identical to a request without JSON mode —
/// JSON mode is opt-in, never a surprise on the main loop's request.
#[test]
fn text_request_omits_response_format_on_wire() {
    let body = OpenAiRequest::from_request(
        request_with(ResponseFormat::Text, ReasoningEffort::Default),
        0.1,
        4096,
    );
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        !json.contains("response_format"),
        "text request must not carry response_format: {json}"
    );
}

/// `Minimal` is the one `reasoning_effort` value outside the
/// `{low, medium, high}` set that the OpenAI-family wire spells, and that
/// spelling is not universally honoured: a Fireworks-backed gateway rejects
/// `reasoning_effort: "minimal"` with HTTP 400 while accepting `low` and an
/// omitted field. The `ReasoningEffort` contract (see `chat.rs`) is that a
/// provider which cannot honour a variant **drops it, never errors** — so
/// the OpenAI wire must omit `reasoning_effort` for `Minimal`, the same as
/// `Default`. This is the regression test for the bug in
/// databook-studio/saya-cli#56: a `Minimal` extraction request no longer
/// hard-fails extraction every turn on a gateway that rejects the spelling.
/// It fails against the current code, which emits `"minimal"`.
#[test]
fn minimal_effort_request_omits_reasoning_effort_on_wire() {
    let body = OpenAiRequest::from_request(
        request_with(ResponseFormat::Text, ReasoningEffort::Minimal),
        0.1,
        4096,
    );
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        !json.contains("reasoning_effort"),
        "minimal effort must not carry reasoning_effort on the OpenAI wire \
         (gateways that reject 'minimal' would 400 every extraction call): {json}"
    );
}

/// A `Default` effort request omits `reasoning_effort` entirely, so the
/// default path sends nothing and the endpoint's own configuration wins —
/// never a surprise on the main loop's request.
#[test]
fn default_effort_request_omits_reasoning_effort_on_wire() {
    let body = OpenAiRequest::from_request(
        request_with(ResponseFormat::Text, ReasoningEffort::Default),
        0.1,
        4096,
    );
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        !json.contains("reasoning_effort"),
        "default effort must not carry reasoning_effort: {json}"
    );
}

/// The cache key derives from the session-stable system message, so two
/// requests that differ only in the last user message share a key and let a
/// caching gateway reuse the cached prefix — the property the prompt cache
/// depends on. The per-turn SQL hint rides the user turn (a context block),
/// so a new question or refined hint never perturbs the key.
#[test]
fn prompt_cache_key_is_stable_across_requests_differing_only_in_last_user_message() {
    let system = ChatMessage::text("system", "SAYA system prompt (session-stable)");
    let req_a = OpenAiRequest::from_request(
        ChatRequest::new(
            "test-model",
            vec![system.clone(), ChatMessage::text("user", "first question")],
        ),
        0.1,
        4096,
    );
    let req_b = OpenAiRequest::from_request(
        ChatRequest::new(
            "test-model",
            vec![
                system.clone(),
                ChatMessage::text("user", "follow-up question"),
            ],
        ),
        0.1,
        4096,
    );
    assert_eq!(
        req_a.prompt_cache_key, req_b.prompt_cache_key,
        "requests differing only in the user turn must share a cache key"
    );
    assert!(
        req_a.prompt_cache_key.is_some(),
        "a request with a system message must produce a cache key"
    );
}

/// The cache key is sensitive to the system message: a different
/// session-stable system prompt yields a different key, so a gateway does
/// not cross-share prefixes between sessions with different connections or
/// memory mode.
#[test]
fn prompt_cache_key_changes_when_the_system_message_changes() {
    let a = OpenAiRequest::from_request(
        ChatRequest::new(
            "test-model",
            vec![ChatMessage::text("system", "system prompt A")],
        ),
        0.1,
        4096,
    );
    let b = OpenAiRequest::from_request(
        ChatRequest::new(
            "test-model",
            vec![ChatMessage::text("system", "system prompt B")],
        ),
        0.1,
        4096,
    );
    assert_ne!(
        a.prompt_cache_key, b.prompt_cache_key,
        "different system messages must yield different cache keys"
    );
}

/// A request with no system message carries no cache key: there is no
/// session-stable prefix for a gateway to cache.
#[test]
fn prompt_cache_key_is_none_without_a_system_message() {
    let body = OpenAiRequest::from_request(
        ChatRequest::new("test-model", vec![ChatMessage::text("user", "hi")]),
        0.1,
        4096,
    );
    assert!(body.prompt_cache_key.is_none());
}
