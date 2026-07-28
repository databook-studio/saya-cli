use super::{
    http::send_stream,
    openai_stream,
    settings::{ProviderSettings, endpoint},
    wire::{messages, tools},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderStream,
};
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
        self.collect(request).await
    }
    async fn stream(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let body = OpenAiRequest::from_request(request);
        let url = endpoint(
            self.settings.base_url.as_deref(),
            "https://api.openai.com/v1",
            "chat/completions",
        );
        let client = &self.client;
        let key = self.api_key.as_deref();
        let response = send_stream(
            || {
                let request = client.post(&url).json(&body);
                if let Some(value) = key {
                    request.bearer_auth(value)
                } else {
                    request
                }
            },
            &self.settings.retry_delays,
            &cancellation,
        )
        .await?;
        Ok(openai_stream::parse(response, cancellation))
    }
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<super::wire::WireMessage>,
    tools: Vec<super::wire::WireTool>,
    stream: bool,
}
impl OpenAiRequest {
    fn from_request(request: ChatRequest) -> Self {
        Self {
            model: request.model,
            messages: messages(request.messages),
            tools: tools(request.tools),
            stream: true,
        }
    }
}
