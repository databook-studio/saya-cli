use super::openai_response::OpenAiResponse;
use super::{
    settings::{ProviderSettings, endpoint},
    wire::{messages, tools},
};
use crate::{ChatProvider, ChatRequest, ChatResponse, ProviderError};
use async_trait::async_trait;
use serde::Serialize;

pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    settings: ProviderSettings,
    api_key: Option<String>,
}

impl OpenAiCompatibleProvider {
    pub fn new(settings: ProviderSettings, api_key: Option<&str>) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .timeout(settings.timeout)
            .build()
            .map_err(|_| ProviderError::Configuration("HTTP client unavailable".into()))?;
        Ok(Self {
            client,
            settings,
            api_key: api_key.map(str::to_owned),
        })
    }
}

#[async_trait]
impl ChatProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = OpenAiRequest::from_request(request);
        let url = endpoint(
            self.settings.base_url.as_deref(),
            "https://api.openai.com/v1",
            "chat/completions",
        );
        let mut request = self.client.post(url).json(&body);
        if let Some(key) = self.api_key.as_deref() {
            request = request.bearer_auth(key);
        }
        let response = request
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
            .json::<OpenAiResponse>()
            .await
            .map_err(|_| ProviderError::InvalidResponse)?;
        payload.into_chat_response()
    }
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<super::wire::WireMessage>,
    tools: Vec<super::wire::WireTool>,
}

impl OpenAiRequest {
    fn from_request(request: ChatRequest) -> Self {
        Self {
            model: request.model,
            messages: messages(request.messages),
            tools: tools(request.tools),
        }
    }
}
