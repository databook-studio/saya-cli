use crate::{ChatRequest, ReasoningEffort};
use serde_json::{Value, json};

/// Anthropic's documented floor for `thinking.budget_tokens`; a smaller budget
/// is refused, so a ceiling with no room for it means no thinking at all.
const MIN_THINKING_BUDGET: u32 = 1024;

pub(super) fn build_body(request: ChatRequest, max_tokens: u32, temperature: Option<f32>) -> Value {
    let mut system_prompts = Vec::new();
    let mut messages = Vec::new();
    let mut pending_tool_results = Vec::new();

    let flush_tool_results = |pending: &mut Vec<Value>, msgs: &mut Vec<Value>| {
        if !pending.is_empty() {
            msgs.push(json!({
                "role": "user",
                "content": std::mem::take(pending)
            }));
        }
    };

    for msg in request.messages {
        if msg.role == "system" {
            if !msg.content.is_empty() {
                system_prompts.push(msg.content);
            }
            continue;
        }

        if msg.role == "tool" {
            let tool_use_id = msg.tool_call_id.as_deref().unwrap_or("");
            pending_tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": msg.content
            }));
            continue;
        }

        flush_tool_results(&mut pending_tool_results, &mut messages);

        if msg.role == "assistant" {
            if msg.tool_calls.is_empty() {
                messages.push(json!({
                    "role": "assistant",
                    "content": msg.content
                }));
            } else {
                let mut content = Vec::new();
                if !msg.content.is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": msg.content
                    }));
                }
                for call in msg.tool_calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments
                    }));
                }
                messages.push(json!({
                    "role": "assistant",
                    "content": content
                }));
            }
        } else {
            messages.push(json!({
                "role": msg.role,
                "content": msg.content
            }));
        }
    }

    flush_tool_results(&mut pending_tool_results, &mut messages);

    let mut body = json!({
        "model": request.model,
        "max_tokens": max_tokens,
        "stream": true,
        "messages": messages
    });

    if !system_prompts.is_empty() {
        // `system` is sent as a single-element array of content blocks rather
        // than a bare string: a string can't carry a `cache_control` breakpoint,
        // but a block can. The block's text is the same string the old form
        // produced (`system_prompts.join("\n\n")`) — only the envelope changes,
        // never the content. Marking the system block caches the stable system
        // prefix across turns of a session.
        body["system"] = json!([{
            "type": "text",
            "text": system_prompts.join("\n\n"),
            "cache_control": {"type": "ephemeral"}
        }]);
    }

    if let Some(temperature) = temperature {
        body["temperature"] = json!(temperature);
    }

    if !request.tools.is_empty() {
        let mut tools: Vec<Value> = request
            .tools
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters
                })
            })
            .collect();
        // Tools are the largest stable block in the request and never change
        // within a session, so the highest-value cache breakpoint is on the
        // **last** tool: everything from the start of the prompt up to and
        // including it is cached. One breakpoint covers the whole tool list.
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }
        body["tools"] = json!(tools);
    }

    // Anthropic's effort lever is a thinking token budget. Each level doubles
    // from the documented minimum —
    // Minimal=1024, Low=2048, Medium=4096, High=8192 — so a higher effort asks for
    // proportionally more thinking. `Default` sends nothing so the endpoint's own
    // configuration wins.
    if request.reasoning_effort != ReasoningEffort::Default {
        let asked = match request.reasoning_effort {
            ReasoningEffort::Minimal => MIN_THINKING_BUDGET,
            ReasoningEffort::Low => 2048,
            ReasoningEffort::Medium => 4096,
            ReasoningEffort::High => 8192,
            ReasoningEffort::Default => 0,
        };
        // Anthropic requires the budget to sit strictly below `max_tokens` and
        // rejects the call otherwise, so cap it to the room available. A ceiling
        // too low for the minimum budget cannot carry thinking at all — omit the
        // field rather than send a request the endpoint will refuse.
        let budget_tokens = asked.min(max_tokens.saturating_sub(1));
        if budget_tokens >= MIN_THINKING_BUDGET {
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget_tokens });
        }
    }

    body
}

#[cfg(test)]
mod tests {
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

    // --- Prompt caching -----------------------------------------------------
    //
    // saya already parses `cache_read_input_tokens` / `cache_creation_input_tokens`
    // from Anthropic responses, but never asked the API to build a cache. These
    // tests pin the request-side breakpoints that make that gauge non-trivial.

    fn tool_def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: "desc".into(),
            read_only: true,
            parameters: json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
        }
    }

    /// Every `cache_control` object in a body, counted recursively. Anthropic
    /// caps a request at four breakpoints; this is the invariant the body must
    /// keep regardless of how many tools or system messages arrive.
    fn cache_breakpoint_count(value: &Value) -> usize {
        match value {
            Value::Object(map) => map
                .iter()
                .map(|(k, v)| {
                    let here = if k == "cache_control" { 1 } else { 0 };
                    here + cache_breakpoint_count(v)
                })
                .sum(),
            Value::Array(items) => items.iter().map(cache_breakpoint_count).sum(),
            _ => 0,
        }
    }

    /// `system` is sent as a single-element array of text blocks so it can carry
    /// a `cache_control` breakpoint. The block's text must equal, byte-for-byte,
    /// the string the old bare-string form produced (`system_prompts.join("\n\n")`)
    /// — only the envelope changes, never the content.
    #[test]
    fn system_serializes_as_one_text_block_with_the_same_text_as_the_old_string() {
        let request = ChatRequest {
            model: "claude-3-5-sonnet".into(),
            messages: vec![
                ChatMessage::text("system", "You are helpful."),
                ChatMessage::text("system", "And careful."),
                ChatMessage::text("user", "hi"),
            ],
            ..Default::default()
        };
        let body = build_body(request, 1024, None);

        let system = body["system"].as_array().expect("system is an array");
        assert_eq!(system.len(), 1, "a single system block: {body}");
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "You are helpful.\n\nAnd careful.");
    }

    /// The system block carries an `ephemeral` cache breakpoint — the marker
    /// that asks Anthropic to cache the stable system prefix.
    #[test]
    fn system_block_carries_an_ephemeral_cache_breakpoint() {
        let request = ChatRequest {
            model: "claude-3-5-sonnet".into(),
            messages: vec![ChatMessage::text("system", "You are helpful.")],
            ..Default::default()
        };
        let body = build_body(request, 1024, None);
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    /// Tools are cached by marking the **last** tool in the array — everything
    /// from the start of the prompt up to and including that block is cached,
    /// so the last tool is the highest-value stable breakpoint. No other tool
    /// carries a marker (one breakpoint covers the whole tool list).
    #[test]
    fn only_the_last_tool_carries_a_cache_breakpoint() {
        let request = ChatRequest {
            model: "claude-3-5-sonnet".into(),
            messages: vec![ChatMessage::text("user", "hi")],
            tools: vec![tool_def("first"), tool_def("second"), tool_def("third")],
            ..Default::default()
        };
        let body = build_body(request, 1024, None);
        let tools = body["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[2]["cache_control"]["type"], "ephemeral");
        assert!(
            tools[0].get("cache_control").is_none(),
            "non-last tool must not carry a breakpoint"
        );
        assert!(
            tools[1].get("cache_control").is_none(),
            "non-last tool must not carry a breakpoint"
        );
    }

    /// A request with no tools still serializes validly and still caches the
    /// system — the system breakpoint stands on its own.
    #[test]
    fn a_request_with_no_tools_still_caches_the_system() {
        let request = ChatRequest {
            model: "claude-3-5-sonnet".into(),
            messages: vec![ChatMessage::text("system", "You are helpful.")],
            tools: Vec::new(),
            ..Default::default()
        };
        let body = build_body(request, 1024, None);
        assert!(
            body.get("tools").is_none(),
            "no tools field when none were supplied: {body}"
        );
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    /// Anthropic caps a request at four `cache_control` breakpoints. With the
    /// stable prefix marked (system + last tool) the body uses two; this test
    /// guards the invariant so a future addition can't silently exceed it.
    #[test]
    fn breakpoint_count_never_exceeds_four() {
        let request = ChatRequest {
            model: "claude-3-5-sonnet".into(),
            messages: vec![ChatMessage::text("system", "You are helpful.")],
            tools: vec![tool_def("a"), tool_def("b"), tool_def("c"), tool_def("d")],
            ..Default::default()
        };
        let body = build_body(request, 1024, None);
        let count = cache_breakpoint_count(&body);
        assert!(
            count >= 1,
            "the stable prefix must be marked with at least one breakpoint, got {count}: {body}"
        );
        assert!(
            count <= 4,
            "Anthropic caps at 4 breakpoints; got {count}: {body}"
        );
    }
}
