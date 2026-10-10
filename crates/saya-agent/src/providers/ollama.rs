use super::{
    http::send_stream,
    ollama_stream,
    ollama_wire::{OllamaRequest, request as ollama_request},
    settings::{ProviderSettings, endpoint},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ControlledProviderError,
    PhysicalAttemptControl, ProviderError, ProviderStream,
};
use async_trait::async_trait;

pub struct OllamaProvider {
    client: reqwest::Client,
    settings: ProviderSettings,
}

impl OllamaProvider {
    pub fn new(settings: ProviderSettings) -> Result<Self, ProviderError> {
        settings.validate_retries()?;
        // No client-wide timeout: streams are bounded per chunk gap instead.
        // Redirects are refused outright: a loopback endpoint classified as
        // local must never replay its POST to another host on a 307.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
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
        self.stream_inner(request, cancellation, None)
            .await
            .map_err(super::http::provider_error)
    }

    async fn stream_with_physical_attempts(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
        attempts: &mut PhysicalAttemptControl<'_>,
    ) -> Result<ProviderStream, ControlledProviderError> {
        self.stream_inner(request, cancellation, Some(attempts))
            .await
    }
}

impl OllamaProvider {
    async fn stream_inner(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
        attempts: Option<&mut PhysicalAttemptControl<'_>>,
    ) -> Result<ProviderStream, ControlledProviderError> {
        let body: OllamaRequest = ollama_request(request).with_options(
            Some(self.settings.temperature),
            Some(self.settings.max_output_tokens),
        );
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
        let build = || client.post(&url).json(&body);
        let response = if let Some(control) = attempts {
            super::http::send_stream_controlled(
                build,
                &self.settings.retry_delays,
                &cancellation,
                &url,
                self.settings.timeout,
                control,
            )
            .await?
        } else {
            send_stream(
                build,
                &self.settings.retry_delays,
                &cancellation,
                &url,
                self.settings.timeout,
            )
            .await?
        };
        Ok(ollama_stream::parse(
            response,
            cancellation,
            self.settings.idle_timeout,
        ))
    }
}
