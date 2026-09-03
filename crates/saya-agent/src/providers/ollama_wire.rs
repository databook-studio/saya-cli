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
    /// Ollama's `options` object — a bag of model parameters. Only
    /// `temperature` is carried, and only when the caller set one, so the
    /// endpoint's own defaults win otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaOptions>,
}

/// Ollama's `options` field: model parameters nested under one object. Only
/// the fields a caller actually set are carried, so an unset parameter never
/// overrides the endpoint's own default.
#[derive(Serialize)]
pub(crate) struct OllamaOptions {
    pub temperature: f32,
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
        options: None,
    }
}

impl OllamaRequest {
    /// Carry sampling temperature under `options.temperature`. `None` leaves
    /// `options` off the wire so the endpoint's own default wins — a value the
    /// caller did not set is never sent.
    pub(crate) fn with_temperature(mut self, temperature: Option<f32>) -> Self {
        self.options = temperature.map(|temperature| OllamaOptions { temperature });
        self
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

    /// Ollama takes sampling temperature under `options.temperature`. A
    /// configured value rides on the wire so an Ollama user's setting reaches
    /// the model — the same `Option<f32>` + emit-only-when-set shape the other
    /// providers use.
    #[test]
    fn temperature_is_carried_under_options_when_configured() {
        let body = request(request_with(ResponseFormat::Text, ReasoningEffort::Default))
            .with_temperature(Some(0.5));
        let value: serde_json::Value = serde_json::to_value(&body).expect("serializes");
        assert_eq!(
            value["options"]["temperature"].as_f64(),
            Some(0.5),
            "configured temperature must ride under options.temperature: {value}"
        );
    }

    /// With no temperature configured, `options` is omitted entirely so the
    /// endpoint's own default wins — a value the caller did not set is never
    /// sent.
    #[test]
    fn options_omitted_when_no_temperature_is_configured() {
        let body = request(request_with(ResponseFormat::Text, ReasoningEffort::Default))
            .with_temperature(None);
        let value: serde_json::Value = serde_json::to_value(&body).expect("serializes");
        assert!(
            value.get("options").is_none(),
            "no temperature configured must not send options: {value}"
        );
    }
}
