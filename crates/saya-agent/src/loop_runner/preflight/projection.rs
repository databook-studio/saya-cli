use crate::{ChatMessage, ReasoningEffort, ResponseFormat, ToolCall, ToolDefinition};
use serde::{Serialize, ser::SerializeSeq};

pub(super) enum Messages<'a> {
    Slice(&'a [ChatMessage]),
    Batch {
        prefix: &'a [ChatMessage],
        assistant: &'a ChatMessage,
        calls: &'a [ToolCall],
        replies: BatchReplies,
    },
}

#[derive(Clone, Copy)]
pub(super) enum BatchReplies {
    Refusal,
    Empty,
}

pub(super) const REFUSAL: &str = r#"{"error":"not executed: request context limit reached"}"#;

#[derive(Serialize)]
struct Projection<'a> {
    model: &'a str,
    messages: Messages<'a>,
    tools: ToolSchemas<'a>,
    response_format: ResponseFormat,
    reasoning_effort: ReasoningEffort,
}

#[derive(Clone, Copy)]
struct ToolSchemas<'a>(&'a [ToolDefinition]);

#[derive(Serialize)]
struct ToolSchema<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Serialize)]
struct ToolReply<'a> {
    role: &'static str,
    content: &'a str,
    tool_call_id: Option<&'a str>,
}

pub(super) enum Measurement {
    Fits(usize),
    Exceeded,
    Serialization,
}

pub(super) fn measure(
    model: &str,
    messages: Messages<'_>,
    definitions: &[ToolDefinition],
    cap: usize,
) -> Measurement {
    match super::counting::measure(
        &Projection {
            model,
            messages,
            tools: ToolSchemas(definitions),
            response_format: ResponseFormat::Text,
            reasoning_effort: ReasoningEffort::Default,
        },
        cap,
    ) {
        Ok(bytes) => Measurement::Fits(bytes),
        Err(super::counting::CountError::Exceeded) => Measurement::Exceeded,
        Err(super::counting::CountError::Serialization) => Measurement::Serialization,
    }
}

impl Serialize for ToolSchemas<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for definition in self.0 {
            sequence.serialize_element(&ToolSchema {
                name: &definition.name,
                description: &definition.description,
                parameters: &definition.parameters,
            })?;
        }
        sequence.end()
    }
}

impl Serialize for Messages<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Slice(messages) => messages.serialize(serializer),
            Self::Batch {
                prefix,
                assistant,
                calls,
                replies,
            } => {
                let mut sequence =
                    serializer.serialize_seq(Some(prefix.len() + 1 + calls.len()))?;
                for message in *prefix {
                    sequence.serialize_element(message)?;
                }
                sequence.serialize_element(assistant)?;
                let content = match replies {
                    BatchReplies::Refusal => REFUSAL,
                    BatchReplies::Empty => "",
                };
                for call in *calls {
                    sequence.serialize_element(&ToolReply {
                        role: "tool",
                        content,
                        tool_call_id: Some(&call.id),
                    })?;
                }
                sequence.end()
            }
        }
    }
}
