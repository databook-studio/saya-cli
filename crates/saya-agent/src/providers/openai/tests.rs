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
        Some(4096),
        super::super::wire::ReplayShape::None,
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
        Some(1234),
        super::super::wire::ReplayShape::None,
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
        Some(4096),
        super::super::wire::ReplayShape::None,
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
/// `Default`. A `Minimal` extraction request must not hard-fail extraction
/// every turn on a gateway that rejects the spelling.
#[test]
fn minimal_effort_request_omits_reasoning_effort_on_wire() {
    let body = OpenAiRequest::from_request(
        request_with(ResponseFormat::Text, ReasoningEffort::Minimal),
        0.1,
        Some(4096),
        super::super::wire::ReplayShape::None,
    );
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        !json.contains("reasoning_effort"),
        "minimal effort must not carry reasoning_effort on the OpenAI wire \
         (gateways that reject 'minimal' would 400 every extraction call): {json}"
    );
}

/// OpenRouter takes `reasoning: {"effort": ...}` and rejects the flat
/// `reasoning_effort` spelling, so the OpenRouter route emits only the object.
#[test]
fn openrouter_route_spells_effort_as_a_reasoning_object() {
    let body = OpenAiRequest::from_request(
        request_with(ResponseFormat::Text, ReasoningEffort::High),
        0.1,
        None,
        super::super::wire::ReplayShape::OpenRouter,
    );
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        json.contains(r#""reasoning":{"effort":"high"}"#),
        "OpenRouter takes its own effort object: {json}"
    );
    assert!(
        !json.contains("reasoning_effort"),
        "OpenRouter must not receive the flat spelling: {json}"
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
        Some(4096),
        super::super::wire::ReplayShape::None,
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
        Some(4096),
        super::super::wire::ReplayShape::None,
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
        Some(4096),
        super::super::wire::ReplayShape::None,
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
        Some(4096),
        super::super::wire::ReplayShape::None,
    );
    let b = OpenAiRequest::from_request(
        ChatRequest::new(
            "test-model",
            vec![ChatMessage::text("system", "system prompt B")],
        ),
        0.1,
        Some(4096),
        super::super::wire::ReplayShape::None,
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
        Some(4096),
        super::super::wire::ReplayShape::None,
    );
    assert!(body.prompt_cache_key.is_none());
}

/// The OpenAI-compatible wire has no image part on a `tool` message, so an
/// image rides a following `user` message — the spec's shape that accepts an
/// image part — instead of being dropped. A text-only batch is untouched: the
/// tool message keeps its bare string content and no message is added.
#[test]
fn an_image_on_a_tool_result_rides_a_following_user_message() {
    let text = r#"{"content":"image/png, 70 bytes"}"#;
    let request = ChatRequest {
        model: "gpt-4o".into(),
        messages: vec![ChatMessage {
            role: "tool".into(),
            content: text.into(),
            tool_calls: vec![],
            tool_call_id: Some("call_1".into()),
            images: vec![crate::ImageBlock {
                media_type: "image/png".into(),
                data_base64: "AAAA".into(),
            }],
            reasoning: None,
            reasoning_signature: None,
        }],
        ..Default::default()
    };

    let body =
        OpenAiRequest::from_request(request, 0.1, None, super::super::wire::ReplayShape::None);
    let messages = serde_json::to_value(&body.messages).expect("serializes");

    assert_eq!(messages[0]["role"], "tool");
    assert_eq!(messages[0]["content"], text);
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"][1]["type"], "image_url");
    assert_eq!(
        messages[1]["content"][1]["image_url"]["url"],
        "data:image/png;base64,AAAA"
    );
    assert_eq!(
        messages.as_array().map(Vec::len),
        Some(2),
        "exactly one follow-up message per image-carrying result: {messages}"
    );
}

/// A DeepSeek-style compatible route replays the last assistant
/// turn's reasoning as `reasoning_content`, and nothing else.
#[test]
fn compatible_route_replays_reasoning_as_reasoning_content() {
    let request = ChatRequest::new(
        "deepseek-chat",
        vec![
            ChatMessage::text("user", "why?"),
            ChatMessage::text("assistant", "because").with_reasoning(Some("because X".into())),
        ],
    );
    let body = OpenAiRequest::from_request(
        request,
        0.1,
        None,
        super::super::wire::ReplayShape::ReasoningContent,
    );
    let messages = serde_json::to_value(&body.messages).expect("serializes");
    assert_eq!(messages[1]["reasoning_content"], "because X");
    assert!(messages[1].get("reasoning").is_none());
    assert!(messages[1].get("reasoning_details").is_none());
    assert!(
        messages[0].get("reasoning_content").is_none(),
        "only the assistant message carries reasoning"
    );
}

/// OpenRouter takes the `reasoning` string plus a structured
/// `reasoning_details` entry, never `reasoning_content`.
#[test]
fn openrouter_route_replays_reasoning_in_its_own_shape() {
    let request = ChatRequest::new(
        "deepseek/deepseek-chat",
        vec![ChatMessage::text("assistant", "because").with_reasoning(Some("because X".into()))],
    );
    let body = OpenAiRequest::from_request(
        request,
        0.1,
        None,
        super::super::wire::ReplayShape::OpenRouter,
    );
    let messages = serde_json::to_value(&body.messages).expect("serializes");
    assert_eq!(messages[0]["reasoning"], "because X");
    assert_eq!(
        messages[0]["reasoning_details"],
        serde_json::json!([{
            "type": "reasoning.text",
            "text": "because X",
            "format": "unknown",
            "index": 0
        }])
    );
    assert!(messages[0].get("reasoning_content").is_none());
}

/// A route with no replay field (native OpenAI, o-series) sends none.
#[test]
fn a_route_without_replay_sends_no_reasoning_fields() {
    let request = ChatRequest::new(
        "o3-mini",
        vec![ChatMessage::text("assistant", "because").with_reasoning(Some("because X".into()))],
    );
    let body =
        OpenAiRequest::from_request(request, 0.1, None, super::super::wire::ReplayShape::None);
    let messages = serde_json::to_value(&body.messages).expect("serializes");
    for field in ["reasoning_content", "reasoning", "reasoning_details"] {
        assert!(messages[0].get(field).is_none(), "{field} must be absent");
    }
}

/// A result with no image adds no message and keeps the exact wire form the
/// text path always produced.
#[test]
fn a_result_without_an_image_adds_no_follow_up_message() {
    let request = ChatRequest {
        model: "gpt-4o".into(),
        messages: vec![ChatMessage::text("user", "hello")],
        ..Default::default()
    };
    let body =
        OpenAiRequest::from_request(request, 0.1, None, super::super::wire::ReplayShape::None);
    let messages = serde_json::to_value(&body.messages).expect("serializes");
    assert_eq!(messages.as_array().map(Vec::len), Some(1));
    assert_eq!(messages[0]["content"], "hello");
}

/// A request with no tools omits the field entirely. An empty `tools` array is
/// not a valid request: a read-only job that composes no tool must send no
/// `tools` field at all.
#[test]
fn request_without_tools_omits_the_tools_field() {
    let mut request = request_with(ResponseFormat::Text, ReasoningEffort::Default);
    request.tools.clear();
    let body = OpenAiRequest::from_request(
        request,
        0.1,
        Some(4096),
        super::super::wire::ReplayShape::None,
    );
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        !json.contains(r#""tools""#),
        "a request with no tools must omit the field: {json}"
    );
}
