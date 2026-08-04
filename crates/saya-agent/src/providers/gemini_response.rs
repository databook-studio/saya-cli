use crate::{ChatMessage, ChatResponse, ProviderError, ToolCall};
use serde_json::Value;

pub(super) fn parse(body: Value) -> Result<ChatResponse, ProviderError> {
    let parts = body
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(|parts| parts.as_array())
        .ok_or(ProviderError::InvalidResponse)?;

    if parts.is_empty() {
        return Err(ProviderError::InvalidResponse);
    }

    let mut content = String::new();
    let mut tool_calls = Vec::new();

    for part in parts {
        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            content.push_str(text);
        }
        if let Some(name) = part
            .get("functionCall")
            .and_then(|fc| fc.get("name"))
            .and_then(|n| n.as_str())
        {
            let fc = &part["functionCall"];
            let args = fc
                .get("args")
                .filter(|a| a.is_object())
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            tool_calls.push(ToolCall {
                id: format!("gemini-{name}"),
                name: name.to_string(),
                arguments: args,
            });
        }
    }

    if content.is_empty() && tool_calls.is_empty() {
        return Err(ProviderError::InvalidResponse);
    }

    Ok(ChatResponse {
        message: ChatMessage {
            role: "assistant".into(),
            content,
            tool_calls,
            tool_call_id: None,
        },
    })
}

#[cfg(test)]
mod tests {
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
}
