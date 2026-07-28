use super::{
    ollama_wire::{OllamaRequest, request as ollama_request},
    settings::{ProviderSettings, endpoint},
};
use crate::{ChatMessage, ChatProvider, ChatRequest, ChatResponse, ProviderError, ToolCall};
use async_trait::async_trait;
use serde::Deserialize;

pub struct OllamaProvider {
    client: reqwest::Client,
    settings: ProviderSettings,
}

impl OllamaProvider {
    pub fn new(settings: ProviderSettings) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .timeout(settings.timeout)
            .build()
            .map_err(|_| ProviderError::Configuration("HTTP client unavailable".into()))?;
        Ok(Self { client, settings })
    }
}

#[async_trait]
impl ChatProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body: OllamaRequest = ollama_request(request);
        let base = self
            .settings
            .base_url
            .as_deref()
            .unwrap_or("http://localhost:11434");
        let suffix = if base.trim_end_matches('/').ends_with("/api") {
            "chat"
        } else {
            "api/chat"
        };
        let response = self
            .client
            .post(endpoint(Some(base), "", suffix))
            .json(&body)
            .send()
            .await
            .map_err(|_| ProviderError::Request("network request failed".into()))?;
        if !response.status().is_success() {
            return Err(ProviderError::Request(format!(
                "HTTP {}",
                response.status().as_u16()
            )));
        }
        let payload = response
            .json::<OllamaResponse>()
            .await
            .map_err(|_| ProviderError::InvalidResponse)?;
        let calls = payload
            .message
            .tool_calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| {
                let arguments = match call.function.arguments {
                    serde_json::Value::String(value) => {
                        serde_json::from_str(&value).map_err(|_| ProviderError::InvalidResponse)?
                    }
                    value => value,
                };
                Ok(ToolCall {
                    id: format!("call-{index}"),
                    name: call.function.name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        Ok(ChatResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content: payload.message.content,
                tool_calls: calls,
                tool_call_id: None,
            },
        })
    }
}

#[derive(Deserialize)]
struct OllamaResponse {
    message: OllamaMessage,
}

#[derive(Deserialize)]
struct OllamaMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

#[derive(Deserialize)]
struct OllamaToolCall {
    function: OllamaFunction,
}

#[derive(Deserialize)]
struct OllamaFunction {
    name: String,
    arguments: serde_json::Value,
}
