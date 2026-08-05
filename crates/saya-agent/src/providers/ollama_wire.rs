use super::wire::{WireTool, tools};
use crate::{ChatMessage, ChatRequest, ToolCall};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct OllamaRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    pub tools: Vec<WireTool>,
    pub stream: bool,
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
