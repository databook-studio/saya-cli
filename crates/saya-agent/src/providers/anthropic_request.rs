use crate::ChatRequest;
use serde_json::{Value, json};

pub(super) fn build_body(request: ChatRequest, max_tokens: u32) -> Value {
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
        body["system"] = json!(system_prompts.join("\n\n"));
    }

    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
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
        body["tools"] = json!(tools);
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, ToolCall, ToolDefinition};

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
                requires_approval: false,
            }],
        };

        let body = build_body(request, 1024);

        assert_eq!(body["system"], "You are helpful.");
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
}
