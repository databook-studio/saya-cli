use crate::ChatRequest;
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) fn build_body(request: ChatRequest) -> Value {
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

    let mut body = json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": 4096
        }
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
    use crate::{ChatMessage, ToolCall, ToolDefinition};

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
                requires_approval: false,
            }],
        };

        let body = build_body(request);

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
}
