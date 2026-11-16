use crate::{ChatMessage, ToolCall, ToolDefinition};
use serde::Serialize;

/// How an OpenAI-compatible route accepts an assistant turn's reasoning.
/// Anthropic has its own signed thinking blocks and is never replayed here;
/// Gemini and Ollama have no replay field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplayShape {
    /// No reasoning field on the wire.
    None,
    /// DeepSeek-style `reasoning_content`.
    ReasoningContent,
    /// OpenRouter's `reasoning` string plus `reasoning_details`.
    OpenRouter,
}

#[derive(Serialize)]
pub(crate) struct WireMessage {
    pub role: String,
    pub content: WireContent,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<WireToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The assistant turn's reasoning on a DeepSeek-style route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// The assistant turn's reasoning on OpenRouter's string field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// OpenRouter's structured companion to `reasoning`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_details: Option<Vec<ReasoningDetail>>,
}

#[derive(Serialize)]
pub(crate) struct ReasoningDetail {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
    pub format: &'static str,
    pub index: u32,
}

/// A message's content on the OpenAI-compatible wire: the bare string every
/// text message sends, or the content-part array an image needs. `untagged`,
/// so a text message serializes exactly as it did as a plain string.
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum WireContent {
    Text(String),
    Parts(Vec<serde_json::Value>),
}

impl WireContent {
    /// The text of a string content, and the empty string for a part array —
    /// the context-block leak tests read messages back through this.
    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Parts(_) => "",
        }
    }
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

pub(crate) fn messages(values: Vec<ChatMessage>, replay: ReplayShape) -> Vec<WireMessage> {
    let mut wire = Vec::with_capacity(values.len());
    for message in values {
        let (reasoning_content, reasoning, reasoning_details) =
            replay_fields(replay, &message.role, message.reasoning);
        wire.push(WireMessage {
            role: message.role,
            content: WireContent::Text(message.content),
            tool_calls: message.tool_calls.into_iter().map(tool_call).collect(),
            tool_call_id: message.tool_call_id,
            reasoning_content,
            reasoning,
            reasoning_details,
        });
        // The `tool` and `assistant` roles carry text only on this wire, so an
        // image cannot ride the message it belongs to. It rides a following
        // `user` message — the shape that accepts an image part — instead of
        // being dropped, and the message it came from keeps its text.
        if !message.images.is_empty() {
            let mut parts = vec![serde_json::json!({
                "type": "text",
                "text": "[saya: the image from the tool result above]"
            })];
            parts.extend(message.images.iter().map(|image| {
                serde_json::json!({
                    "type": "image_url",
                    "image_url": {"url": super::images::data_url(image)}
                })
            }));
            wire.push(WireMessage {
                role: "user".into(),
                content: WireContent::Parts(parts),
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: None,
                reasoning: None,
                reasoning_details: None,
            });
        }
    }
    wire
}

/// The replay fields for one message. Only the assistant turn's reasoning is
/// ever sent; a tool or user message cannot carry it. The message's reasoning
/// is already bounded by `ChatMessage::with_reasoning`.
fn replay_fields(
    replay: ReplayShape,
    role: &str,
    reasoning: Option<String>,
) -> (Option<String>, Option<String>, Option<Vec<ReasoningDetail>>) {
    let Some(text) = reasoning.filter(|text| role == "assistant" && !text.is_empty()) else {
        return (None, None, None);
    };
    match replay {
        ReplayShape::None => (None, None, None),
        ReplayShape::ReasoningContent => (Some(text), None, None),
        ReplayShape::OpenRouter => {
            let detail = ReasoningDetail {
                kind: "reasoning.text",
                text: text.clone(),
                format: "unknown",
                index: 0,
            };
            (None, Some(text), Some(vec![detail]))
        }
    }
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
