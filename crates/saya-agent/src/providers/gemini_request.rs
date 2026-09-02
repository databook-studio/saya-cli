use crate::{ChatRequest, ReasoningEffort};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) fn build_body(
    request: ChatRequest,
    max_output_tokens: u32,
    temperature: Option<f32>,
) -> Value {
    let mut system_prompts = Vec::new();
    let mut contents = Vec::new();
    let mut tool_name_map = HashMap::new();

    for msg in request.messages {
        if msg.role == "system" {
            if !msg.content.is_empty() {
                system_prompts.push(msg.content);
            }
            continue;
        }

        if msg.role == "assistant" {
            for call in &msg.tool_calls {
                tool_name_map.insert(call.id.clone(), call.name.clone());
            }

            let mut parts = Vec::new();
            if !msg.content.is_empty() {
                parts.push(json!({ "text": msg.content }));
            }
            for call in msg.tool_calls {
                parts.push(json!({
                    "functionCall": {
                        "name": call.name,
                        "args": call.arguments
                    }
                }));
            }
            contents.push(json!({
                "role": "model",
                "parts": parts
            }));
            continue;
        }

        if msg.role == "tool" {
            let id_str = msg.tool_call_id.as_deref().unwrap_or("");
            let resolved_name = tool_name_map
                .get(id_str)
                .map(String::as_str)
                .unwrap_or(id_str);
            contents.push(json!({
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": resolved_name,
                        "response": {
                            "result": msg.content
                        }
                    }
                }]
            }));
            continue;
        }

        contents.push(json!({
            "role": "user",
            "parts": [{ "text": msg.content }]
        }));
    }

    let mut generation_config = json!({
        "maxOutputTokens": max_output_tokens
    });
    if let Some(temperature) = temperature {
        generation_config["temperature"] = json!(temperature);
    }
    // Gemini's effort lever is a thinking token budget. `0` disables thinking
    // (Gemini API docs); `Minimal` maps to `0` (the least-thinking lever Gemini
    // offers), then conservative steps — Low=1024, Medium=4096, High=8192.
    // `Default` sends nothing so the endpoint's own configuration wins.
    if request.reasoning_effort != ReasoningEffort::Default {
        let thinking_budget = match request.reasoning_effort {
            ReasoningEffort::Minimal => 0,
            ReasoningEffort::Low => 1024,
            ReasoningEffort::Medium => 4096,
            ReasoningEffort::High => 8192,
            ReasoningEffort::Default => 0,
        };
        generation_config["thinkingConfig"] = json!({ "thinkingBudget": thinking_budget });
    }
    let mut body = json!({
        "contents": contents,
        "generationConfig": generation_config
    });

    if !system_prompts.is_empty() {
        body["systemInstruction"] = json!({
            "parts": [{ "text": system_prompts.join("\n\n") }]
        });
    }

    if !request.tools.is_empty() {
        let declarations: Vec<Value> = request
            .tools
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters
                })
            })
            .collect();
        body["tools"] = json!([{ "functionDeclarations": declarations }]);
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, LocalStateEffect, ToolCall, ToolDefinition, ToolEffect};

    #[test]
    fn test_gemini_request_mapping_and_tool_resolution() {
        let request = ChatRequest {
            model: "gemini-1.5-flash".into(),
            messages: vec![
                ChatMessage::text("system", "System prompt text"),
                ChatMessage {
                    role: "assistant".into(),
                    content: "Let me search.".into(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "search".into(),
                        arguments: json!({"query": "rust"}),
                    }],
                    tool_call_id: None,
                },
                ChatMessage {
                    role: "tool".into(),
                    content: "Search result content".into(),
                    tool_calls: vec![],
                    tool_call_id: Some("c1".into()),
                },
            ],
            tools: vec![ToolDefinition {
                name: "search".into(),
                description: "Search the web".into(),
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

        let body = build_body(request, 4096, None);

        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "System prompt text"
        );

        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 2);

        assert_eq!(contents[0]["role"], "model");
        let model_parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(model_parts[0]["text"], "Let me search.");
        assert_eq!(model_parts[1]["functionCall"]["name"], "search");
        assert_eq!(model_parts[1]["functionCall"]["args"]["query"], "rust");

        assert_eq!(contents[1]["role"], "user");
        let user_parts = contents[1]["parts"].as_array().unwrap();
        assert_eq!(user_parts[0]["functionResponse"]["name"], "search");
        assert_eq!(
            user_parts[0]["functionResponse"]["response"]["result"],
            "Search result content"
        );

        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "search"
        );
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 4096);
    }

    /// Q2: Gemini shapes output with `responseMimeType`, but this provider
    /// deliberately ignores the JSON hint for now and never emits it — even when
    /// the caller asked for `JsonObject`. Ignoring degrades to today's behaviour
    /// (the prompt already asks for JSON, `strip_markdown_fences` handles
    /// fences), never to an error (invariant 3). This test pins the
    /// "deliberately omits" decision so a future change has to reconsider it
    /// consciously.
    #[test]
    fn json_hint_is_deliberately_omitted_from_gemini_body() {
        let request = ChatRequest {
            model: "gemini-1.5-flash".into(),
            messages: vec![
                ChatMessage::text("system", "extract"),
                ChatMessage::text("user", "proposals"),
            ],
            tools: Vec::new(),
            response_format: crate::ResponseFormat::JsonObject,
            reasoning_effort: ReasoningEffort::Default,
        };
        let body = build_body(request, 4096, None);
        assert!(
            body["generationConfig"].get("responseMimeType").is_none(),
            "gemini must not emit responseMimeType: {}",
            body["generationConfig"]
        );
    }

    /// Gemini's effort lever is a thinking token budget. `Minimal` carries
    /// `thinkingConfig: {thinkingBudget: 0}` — `0` disables thinking (Gemini API
    /// docs), the least-thinking lever Gemini offers.
    #[test]
    fn minimal_effort_request_carries_thinking_budget_zero_on_wire() {
        let request = ChatRequest {
            model: "gemini-1.5-flash".into(),
            messages: vec![ChatMessage::text("user", "extract")],
            tools: Vec::new(),
            reasoning_effort: ReasoningEffort::Minimal,
            ..Default::default()
        };
        let body = build_body(request, 4096, None);
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            0
        );
    }

    /// Q2: a `Default` effort request omits `thinkingConfig` entirely, so the
    /// default path sends nothing and the endpoint's own configuration wins.
    #[test]
    fn default_effort_request_omits_thinking_config_on_wire() {
        let request = ChatRequest {
            model: "gemini-1.5-flash".into(),
            messages: vec![ChatMessage::text("user", "extract")],
            tools: Vec::new(),
            ..Default::default()
        };
        let body = build_body(request, 4096, None);
        assert!(
            body["generationConfig"].get("thinkingConfig").is_none(),
            "default effort must not emit thinkingConfig: {}",
            body["generationConfig"]
        );
    }
}
