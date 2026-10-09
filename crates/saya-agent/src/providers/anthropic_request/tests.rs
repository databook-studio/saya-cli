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
            },
            ChatMessage {
                role: "tool".into(),
                content: "Rainy, 15C".into(),
                tool_calls: vec![],
                tool_call_id: Some("call_1".into()),
            },
            ChatMessage {
                role: "tool".into(),
                content: "Humidity 80%".into(),
                tool_calls: vec![],
                tool_call_id: Some("call_1".into()),
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

    let body = build_body(request, 1024, None);

    // `system` is serialized as a single-element array of text blocks (the
    // envelope that lets it carry a `cache_control` breakpoint); the block's
    // text is the same string the old bare-string form produced.
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
/// `JsonObject`. Ignoring degrades to today's behaviour (the prompt already
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
    let body = build_body(request, 1024, None);
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
    let body = build_body(request, 4096, None);
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
        reasoning_effort: ReasoningEffort::High,
        ..Default::default()
    };
    let body = build_body(request, 4096, None);
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
    let body = build_body(request, 1024, None);
    assert!(
        body.get("thinking").is_none(),
        "a ceiling with no room for the minimum budget must omit thinking: {body}"
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
    let body = build_body(request, 1024, None);
    assert!(
        body.get("thinking").is_none(),
        "default effort must not emit thinking: {}",
        body
    );
}

#[path = "cache_tests.rs"]
mod cache_tests;
