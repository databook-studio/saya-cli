use super::{gemini_request, gemini_response, settings::ProviderSettings};
use crate::{CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError};
use async_trait::async_trait;

/// Gemini API provider implementation.
pub struct GeminiProvider {
    client: reqwest::Client,
    settings: ProviderSettings,
    api_key: Option<String>,
}

impl GeminiProvider {
    /// Creates a new `GeminiProvider` with the given settings and optional API key.
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
impl ChatProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let model = request.model.clone();
        let body = gemini_request::build_body(request);
        let root = self
            .settings
            .base_url
            .as_deref()
            .unwrap_or("https://generativelanguage.googleapis.com/v1beta")
            .trim_end_matches('/');
        let url = format!("{root}/models/{model}:generateContent");
        let cancellation = CancellationToken::new();
        let client = &self.client;
        let key = self.api_key.as_deref();

        let response = super::http::send_stream(
            || {
                let r = client.post(&url).json(&body);
                if let Some(k) = key {
                    r.header("x-goog-api-key", k)
                } else {
                    r
                }
            },
            &self.settings.retry_delays,
            &cancellation,
        )
        .await?;

        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| ProviderError::InvalidResponse)?;

        gemini_response::parse(value)
    }
}
