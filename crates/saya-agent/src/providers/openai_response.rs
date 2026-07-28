use crate::{ChatMessage, ChatResponse, ProviderError, ToolCall};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCall>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    id: Option<String>,
    function: OpenAiFunction,
}

#[derive(Deserialize)]
struct OpenAiFunction {
    name: Option<String>,
    arguments: serde_json::Value,
}

impl OpenAiResponse {
    pub(super) fn into_chat_response(self) -> Result<ChatResponse, ProviderError> {
        let message = self
            .choices
            .into_iter()
            .next()
            .ok_or(ProviderError::InvalidResponse)?
            .message;
        let tool_calls = message
            .tool_calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| {
                let name = call.function.name.ok_or(ProviderError::InvalidResponse)?;
                let arguments = normalize_arguments(call.function.arguments)?;
                Ok(ToolCall {
                    id: call.id.unwrap_or_else(|| format!("call-{index}")),
                    name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        let content = message.content.unwrap_or_default();
        if content.trim().is_empty() && tool_calls.is_empty() {
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
}

fn normalize_arguments(value: serde_json::Value) -> Result<serde_json::Value, ProviderError> {
    match value {
        serde_json::Value::String(value) => {
            serde_json::from_str(&value).map_err(|_| ProviderError::InvalidResponse)
        }
        value => Ok(value),
    }
}
