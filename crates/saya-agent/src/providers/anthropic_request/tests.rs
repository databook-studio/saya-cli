use super::*;
use crate::{ChatMessage, LocalStateEffect, ToolCall, ToolDefinition, ToolEffect};

#[test]
fn test_message_mapping_and_system_extraction() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![
            ChatMessage::text("system", "You are helpful."),
            ChatMessage {
                role: "assistant".into(),
                content: "".into(),
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "get_weather".into(),
                    arguments: json!({"location": "London"}),
                }],
                tool_call_id: None,

                images: Vec::new(),
                reasoning: None,
                reasoning_signature: None,
            },
            ChatMessage {
                role: "tool".into(),
                content: "Rainy, 15C".into(),
                tool_calls: vec![],
                tool_call_id: Some("call_1".into()),

                images: Vec::new(),
                reasoning: None,
                reasoning_signature: None,
            },
            ChatMessage {
                role: "tool".into(),
                content: "Humidity 80%".into(),
                tool_calls: vec![],
                tool_call_id: Some("call_1".into()),

                images: Vec::new(),
                reasoning: None,
                reasoning_signature: None,
            },
        ],
        tools: vec![ToolDefinition {
            name: "get_weather".into(),
            description: "Get weather".into(),
            read_only: true,
            parameters: json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: crate::ToolConcurrency::Serial,
            completion: None,
        }],
        ..Default::default()
    };

    let body = build_body(request, 1024, None, EffortSource::Stated, false);

    // `system` is serialized as a single-element array of text blocks (the
    // envelope that lets it carry a `cache_control` breakpoint); the block's
    // text is the same string the bare-string form produced.
    assert_eq!(body["system"][0]["type"], "text");
    assert_eq!(body["system"][0]["text"], "You are helpful.");
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);

    // Assistant message with tool_use block
    assert_eq!(messages[0]["role"], "assistant");
    let assistant_content = messages[0]["content"].as_array().unwrap();
    assert_eq!(assistant_content.len(), 1);
    assert_eq!(assistant_content[0]["type"], "tool_use");
    assert_eq!(assistant_content[0]["id"], "call_1");
    assert_eq!(assistant_content[0]["name"], "get_weather");
    assert_eq!(assistant_content[0]["input"]["location"], "London");

    // User message with two tool_result blocks
    assert_eq!(messages[1]["role"], "user");
    let user_content = messages[1]["content"].as_array().unwrap();
    assert_eq!(user_content.len(), 2);
    assert_eq!(user_content[0]["type"], "tool_result");
    assert_eq!(user_content[0]["tool_use_id"], "call_1");
    assert_eq!(user_content[0]["content"], "Rainy, 15C");
    assert_eq!(user_content[1]["type"], "tool_result");
    assert_eq!(user_content[1]["tool_use_id"], "call_1");
    assert_eq!(user_content[1]["content"], "Humidity 80%");

    // Tools present
    assert_eq!(body["tools"][0]["name"], "get_weather");
    assert_eq!(body["tools"][0]["input_schema"], json!({"type": "object"}));
}

/// Anthropic has no direct `response_format` equivalent (it shapes output
/// through tools), so this provider deliberately ignores the JSON hint and
/// never emits a `response_format` field — even when the caller asked for
/// `JsonObject`. Ignoring degrades to the existing behaviour (the prompt already
/// asks for JSON, `strip_markdown_fences` handles fences), never to an error
///. This test pins the "deliberately omits" decision so a
/// future change has to reconsider it consciously.
#[test]
fn json_hint_is_deliberately_omitted_from_anthropic_body() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![
            ChatMessage::text("system", "extract"),
            ChatMessage::text("user", "proposals"),
        ],
        tools: Vec::new(),
        response_format: crate::ResponseFormat::JsonObject,
        reasoning_effort: ReasoningEffort::Default,
    };
    let body = build_body(request, 1024, None, EffortSource::Stated, false);
    assert!(
        body.get("response_format").is_none(),
        "anthropic must not emit response_format: {}",
        body
    );
}

/// Anthropic's effort lever is a thinking token budget. `Minimal` carries
/// `thinking: {type: "enabled", budget_tokens: 1024}` — the documented
/// `budget_tokens` minimum (each higher level doubles from that floor).
#[test]
fn minimal_effort_request_carries_thinking_budget_on_wire() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "extract")],
        tools: Vec::new(),
        reasoning_effort: ReasoningEffort::Minimal,
        ..Default::default()
    };
    let body = build_body(request, 4096, None, EffortSource::Stated, false);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 1024);
}

/// Anthropic rejects a request whose thinking budget is not strictly below
/// `max_tokens`, so the budget is capped to leave room rather than sent as
/// asked. Without this, the default 4096 ceiling makes `Medium` (4096) and
/// `High` (8192) invalid on every call.
#[test]
fn a_thinking_budget_stays_below_the_output_ceiling() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "think hard")],
        tools: Vec::new(),
        reasoning_effort: ReasoningEffort::Medium,
        ..Default::default()
    };
    let body = build_body(request, 4096, None, EffortSource::Stated, false);
    let budget = body["thinking"]["budget_tokens"]
        .as_u64()
        .expect("a budget");
    assert!(
        budget < 4096,
        "budget {budget} must stay under max_tokens or Anthropic rejects the call"
    );
}

/// When the output ceiling leaves no room for Anthropic's 1024-token
/// minimum budget, asking for thinking at all would make the request
/// invalid — so the field is omitted and the call still works.
#[test]
fn no_thinking_field_when_the_ceiling_cannot_fit_the_minimum_budget() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "extract")],
        tools: Vec::new(),
        reasoning_effort: ReasoningEffort::Minimal,
        ..Default::default()
    };
    let body = build_body(request, 1024, None, EffortSource::Stated, false);
    assert!(
        body.get("thinking").is_none(),
        "a ceiling with no room for the minimum budget must omit thinking: {body}"
    );
}

/// High effort is adaptive thinking plus an effort setting, with no temperature
/// and no fixed budget. A budget would bring back the long reasoning traces.
#[test]
fn high_effort_request_is_adaptive_thinking() {
    let request = ChatRequest {
        model: "glm-5.3-flash".into(),
        messages: vec![ChatMessage::text("user", "implement")],
        tools: Vec::new(),
        reasoning_effort: ReasoningEffort::High,
        ..Default::default()
    };
    let body = build_body(request, 128_000, Some(1.0), EffortSource::Stated, false);
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["output_config"]["effort"], "high");
    assert!(
        body.get("thinking")
            .and_then(|v| v.get("budget_tokens"))
            .is_none()
    );
    assert!(body.get("temperature").is_none());
}

/// The loop's adaptive policy uses the provider's own effort control for its
/// routine `low`: adaptive thinking with `output_config.effort: low`, and no
/// temperature. A `low` the operator pinned keeps the fixed budget (see the
/// provider integration test), because it is a stated level, not a hint.
#[test]
fn policy_low_request_is_adaptive_with_low_effort() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "continue")],
        tools: Vec::new(),
        reasoning_effort: ReasoningEffort::Low,
        ..Default::default()
    };
    let body = build_body(request, 32_000, Some(1.0), EffortSource::Policy, false);
    assert_eq!(body["thinking"], json!({"type": "adaptive"}));
    assert_eq!(body["output_config"], json!({"effort": "low"}));
    assert!(
        body.get("temperature").is_none(),
        "adaptive thinking rejects temperature: {body}"
    );
}

/// A `Default` effort request omits the `thinking` field entirely, so
/// the default path sends nothing and the endpoint's own configuration wins.
#[test]
fn default_effort_request_omits_thinking_on_wire() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "extract")],
        tools: Vec::new(),
        ..Default::default()
    };
    let body = build_body(request, 1024, None, EffortSource::Stated, false);
    assert!(
        body.get("thinking").is_none(),
        "default effort must not emit thinking: {}",
        body
    );
}

#[path = "cache_tests.rs"]
mod cache_tests;

/// An image a tool result carried rides that result's own content blocks —
/// the shape Anthropic accepts — so the model sees the image where the result
/// is. A result with no image keeps the bare string content it always had.
#[test]
fn a_tool_result_image_rides_that_results_own_content_blocks() {
    let text = r#"{"content":"image/png, 70 bytes"}"#;
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
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

    let body = build_body(request, 1024, None, EffortSource::Stated, false);

    let result = &body["messages"][0]["content"][0];
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["tool_use_id"], "call_1");
    assert_eq!(result["content"][0]["type"], "text");
    assert_eq!(result["content"][0]["text"], text);
    assert_eq!(result["content"][1]["type"], "image");
    assert_eq!(result["content"][1]["source"]["type"], "base64");
    assert_eq!(result["content"][1]["source"]["media_type"], "image/png");
    assert_eq!(result["content"][1]["source"]["data"], "AAAA");
}

/// A signed thinking block is replayed first, then the reply it produced.
/// Anthropic accepts thinking back only with its signature, so both travel
/// together or not at all.
#[test]
fn a_signed_thinking_block_is_replayed_before_the_reply() {
    let assistant = ChatMessage {
        role: "assistant".into(),
        content: "answer".into(),
        tool_calls: vec![ToolCall {
            id: "call_1".into(),
            name: "get_weather".into(),
            arguments: json!({"location": "London"}),
        }],
        tool_call_id: None,
        images: Vec::new(),
        reasoning: Some("plan".into()),
        reasoning_signature: Some("sig-1".into()),
    };
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "weather?"), assistant],
        tools: Vec::new(),
        ..Default::default()
    };
    let body = build_body(request, 4096, None, EffortSource::Stated, false);
    let content = body["messages"][1]["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[0]["thinking"], "plan");
    assert_eq!(content[0]["signature"], "sig-1");
    assert_eq!(content[1]["type"], "text");
    assert_eq!(content[2]["type"], "tool_use");
}

/// A text-only assistant turn with a signature still uses the block form:
/// a bare string cannot carry the thinking block.
#[test]
fn a_signed_thinking_block_precedes_a_plain_text_reply() {
    let assistant = ChatMessage {
        role: "assistant".into(),
        content: "answer".into(),
        reasoning: Some("plan".into()),
        reasoning_signature: Some("sig-1".into()),
        ..Default::default()
    };
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "hello"), assistant],
        tools: Vec::new(),
        ..Default::default()
    };
    let body = build_body(request, 4096, None, EffortSource::Stated, false);
    let content = body["messages"][1]["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[1]["type"], "text");
    assert_eq!(content[1]["text"], "answer");
}

/// Without a signature the route keeps today's behaviour: the reasoning text
/// is never guessed back into a thinking block.
#[test]
fn thinking_without_a_signature_is_not_replayed() {
    let assistant = ChatMessage {
        role: "assistant".into(),
        content: "answer".into(),
        reasoning: Some("plan".into()),
        ..Default::default()
    };
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "hello"), assistant],
        tools: Vec::new(),
        ..Default::default()
    };
    let body = build_body(request, 4096, None, EffortSource::Stated, false);
    assert_eq!(body["messages"][1]["content"], "answer");
}
