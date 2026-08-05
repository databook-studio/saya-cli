use super::{
    http::send_stream,
    ollama_stream,
    ollama_wire::{OllamaRequest, request as ollama_request},
    settings::{ProviderSettings, endpoint},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderStream,
};
use async_trait::async_trait;

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
        self.collect(request).await
    }
    async fn stream(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
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
        let url = endpoint(Some(base), "", suffix);
        let client = &self.client;
        let response = send_stream(
            || client.post(&url).json(&body),
            &self.settings.retry_delays,
            &cancellation,
        )
        .await?;
        Ok(ollama_stream::parse(response, cancellation))
    }
}
