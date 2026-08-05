use super::{
    anthropic_request, anthropic_stream,
    http::send_stream,
    settings::{ProviderSettings, endpoint},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderStream,
};
use async_trait::async_trait;

/// Anthropic API provider implementation.
pub struct AnthropicProvider {
    client: reqwest::Client,
    settings: ProviderSettings,
    api_key: Option<String>,
}

impl AnthropicProvider {
    /// Creates a new `AnthropicProvider` with the given settings and optional API key.
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
impl ChatProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.collect(request).await
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let body = anthropic_request::build_body(request, 4096);
        let url = endpoint(
            self.settings.base_url.as_deref(),
            "https://api.anthropic.com/v1",
            "messages",
        );
        let client = &self.client;
        let key = self.api_key.as_deref();
        let response = send_stream(
            || {
                let request = client.post(&url).json(&body);
                let request = request.header("anthropic-version", "2023-06-01");
                if let Some(k) = key {
                    request.header("x-api-key", k)
                } else {
                    request
                }
            },
            &self.settings.retry_delays,
            &cancellation,
        )
        .await?;
        Ok(anthropic_stream::parse(response, cancellation))
    }
}
