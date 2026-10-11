use super::{
    anthropic_request, anthropic_stream,
    http::send_stream,
    settings::{ProviderSettings, endpoint},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ControlledProviderError,
    PhysicalAttemptControl, ProviderError, ProviderStream,
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
        settings.validate_retries()?;
        // No client-wide timeout: streaming responses are bounded per chunk
        // gap (`idle_timeout`) so long healthy generations are never killed.
        // Redirects are refused outright: `base_url` is user-configurable, so
        // a misconfigured or hostile endpoint could answer 307 and have the
        // default policy replay the POST to another host — prompt plus
        // database-derived context in the body, and the `x-api-key` header
        // untouched by reqwest's cross-host strip (which removes only
        // `Authorization` and cookie headers).
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
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

impl AnthropicProvider {
    async fn stream_inner(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
        mut attempts: Option<&mut PhysicalAttemptControl<'_>>,
    ) -> Result<ProviderStream, ControlledProviderError> {
        let request_model = request.model.clone();
        let body = anthropic_request::build_body(
            request,
            self.settings.max_output_tokens,
            Some(self.settings.temperature),
        );
        let url = endpoint(
            self.settings.base_url.as_deref(),
            "https://api.anthropic.com/v1",
            "messages",
        );
        if let Some(control) = attempts.as_deref_mut() {
            control.bind_request(crate::PhysicalRequestMetadata::new(
                "anthropic",
                &request_model,
                &url,
                self.settings.max_output_tokens,
            )?)?;
        }
        let client = &self.client;
        let key = self.api_key.as_deref();
        let build = || {
            let request = client.post(&url).json(&body);
            let request = request.header("anthropic-version", "2023-06-01");
            if let Some(k) = key {
                request.header("x-api-key", k)
            } else {
                request
            }
        };
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
        Ok(anthropic_stream::parse(
            response,
            cancellation,
            self.settings.idle_timeout,
        ))
    }
}
