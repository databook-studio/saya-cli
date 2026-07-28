use crate::{ChatMessage, ToolCall, ToolDefinition};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct WireMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<WireToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct WireToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: WireFunction,
}

#[derive(Serialize)]
pub(crate) struct WireFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Serialize)]
pub(crate) struct WireTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: WireToolDefinition,
}

#[derive(Serialize)]
pub(crate) struct WireToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub(crate) fn messages(values: Vec<ChatMessage>) -> Vec<WireMessage> {
    values
        .into_iter()
        .map(|message| WireMessage {
            role: message.role,
            content: message.content,
            tool_calls: message.tool_calls.into_iter().map(tool_call).collect(),
            tool_call_id: message.tool_call_id,
        })
        .collect()
}

pub(crate) fn tools(values: Vec<ToolDefinition>) -> Vec<WireTool> {
    values
        .into_iter()
        .map(|tool| WireTool {
            kind: "function".into(),
            function: WireToolDefinition {
                name: tool.name,
                description: tool.description,
                parameters: tool.parameters,
            },
        })
        .collect()
}

fn tool_call(call: ToolCall) -> WireToolCall {
    WireToolCall {
        id: call.id,
        kind: "function".into(),
        function: WireFunction {
            name: call.name,
            arguments: call.arguments.to_string(),
        },
    }
}
