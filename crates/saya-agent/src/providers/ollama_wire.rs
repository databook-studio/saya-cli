use super::wire::{WireTool, tools};
use crate::{ChatMessage, ChatRequest, ReasoningEffort, ResponseFormat, ToolCall};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct OllamaRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    pub tools: Vec<WireTool>,
    pub stream: bool,
    /// Ollama's `format` field — `"json"` constrains the response to valid
    /// JSON. Only present when the caller asked for JSON (the Ollama spelling of
    /// [`ChatRequest::response_format`]); omitted for `Text` so the default
    /// prose path is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<&'static str>,
    /// Ollama's `think` field — the provider's only effort lever, a boolean.
    /// Omitted for `Default` so the default path sends nothing; `Minimal`/`Low`
    /// disable thinking, `Medium`/`High` enable it (a boolean cannot split four
    /// levels, so the boundary sits at `Medium`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub think: Option<bool>,
}

#[derive(Serialize)]
pub(crate) struct OllamaMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<OllamaToolCall>,
}

#[derive(Serialize)]
pub(crate) struct OllamaToolCall {
    pub function: OllamaFunction,
}

#[derive(Serialize)]
pub(crate) struct OllamaFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

pub(crate) fn request(request: ChatRequest) -> OllamaRequest {
    OllamaRequest {
        model: request.model,
        messages: messages(request.messages),
        tools: tools(request.tools),
        stream: true,
        format: match request.response_format {
            ResponseFormat::JsonObject => Some("json"),
            ResponseFormat::Text => None,
        },
        think: match request.reasoning_effort {
            ReasoningEffort::Default => None,
            ReasoningEffort::Minimal | ReasoningEffort::Low => Some(false),
            ReasoningEffort::Medium | ReasoningEffort::High => Some(true),
        },
    }
}

fn messages(values: Vec<ChatMessage>) -> Vec<OllamaMessage> {
    values
        .into_iter()
        .map(|message| OllamaMessage {
            role: message.role,
            content: message.content,
            tool_calls: message.tool_calls.into_iter().map(tool_call).collect(),
        })
        .collect()
}

fn tool_call(call: ToolCall) -> OllamaToolCall {
    OllamaToolCall {
        function: OllamaFunction {
            name: call.name,
            arguments: call.arguments,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalStateEffect, ToolDefinition, ToolEffect};

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
            }],
            response_format: format,
            reasoning_effort: effort,
        }
    }

    /// A JSON-mode request carries Ollama's `format: "json"` spelling.
    #[test]
    fn json_object_request_carries_format_json_on_wire() {
        let body = request(request_with(
            ResponseFormat::JsonObject,
            ReasoningEffort::Default,
        ));
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            json.contains(r#""format":"json""#),
            "format json must appear on the wire: {json}"
        );
    }

    /// A `Text` (default) request omits `format`, so the prose
    /// path is unchanged.
    #[test]
    fn text_request_omits_format_on_wire() {
        let body = request(request_with(ResponseFormat::Text, ReasoningEffort::Default));
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            !json.contains(r#""format""#),
            "text request must not carry format: {json}"
        );
    }

    /// Ollama's effort lever is a boolean: `Minimal`/`Low` carry `think: false`
    /// (less thinking), `Medium`/`High` carry `think: true`. A boolean cannot
    /// split four levels, so the boundary sits at `Medium`.
    #[test]
    fn minimal_effort_request_carries_think_false_on_wire() {
        let body = request(request_with(ResponseFormat::Text, ReasoningEffort::Minimal));
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            json.contains(r#""think":false"#),
            "minimal effort must carry think:false: {json}"
        );
    }

    #[test]
    fn medium_effort_request_carries_think_true_on_wire() {
        let body = request(request_with(ResponseFormat::Text, ReasoningEffort::Medium));
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            json.contains(r#""think":true"#),
            "medium effort must carry think:true: {json}"
        );
    }

    /// A `Default` effort request omits `think`, so the default path sends
    /// nothing and the endpoint's own configuration wins.
    #[test]
    fn default_effort_request_omits_think_on_wire() {
        let body = request(request_with(ResponseFormat::Text, ReasoningEffort::Default));
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            !json.contains(r#""think""#),
            "default effort must not carry think: {json}"
        );
    }
}
