use super::{gemini_request, gemini_response, settings::ProviderSettings};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ControlledProviderError,
    PhysicalAttemptControl, ProviderError, ProviderEvent, ProviderStream,
};
use async_trait::async_trait;
use futures_util::stream;

/// Gemini API provider implementation.
pub struct GeminiProvider {
    client: reqwest::Client,
    settings: ProviderSettings,
    api_key: Option<String>,
}

impl GeminiProvider {
    /// Creates a new `GeminiProvider` with the given settings and optional API key.
    pub fn new(settings: ProviderSettings, api_key: Option<&str>) -> Result<Self, ProviderError> {
        settings.validate_retries()?;
        // Redirects are refused outright: `base_url` is user-configurable, so
        // a misconfigured or hostile endpoint could answer 307 and have the
        // default policy replay the POST to another host — prompt plus
        // database-derived context in the body, and the `x-goog-api-key`
        // header untouched by reqwest's cross-host strip (which removes only
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
impl ChatProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let value = self
            .fetch_value(request, CancellationToken::new(), None)
            .await
            .map_err(super::http::provider_error)?;
        gemini_response::parse(value)
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

impl GeminiProvider {
    async fn stream_inner(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
        attempts: Option<&mut PhysicalAttemptControl<'_>>,
    ) -> Result<ProviderStream, ControlledProviderError> {
        let value = self.fetch_value(request, cancellation, attempts).await?;
        let response = match gemini_response::parse(value.clone()) {
            Ok(response) => response,
            Err(error @ ProviderError::OutputTruncated { .. }) => {
                // The paid usage of a capped response is delivered ahead of
                // the typed error: the loop charges only usage a stream emits.
                let mut items = Vec::with_capacity(2);
                if let Some(usage) = gemini_response::usage(&value) {
                    items.push(Ok(ProviderEvent::Usage(usage)));
                }
                items.push(Err(error));
                return Ok(Box::pin(stream::iter(items)));
            }
            Err(error) => return Err(error.into()),
        };
        let mut events = if response.message.tool_calls.is_empty() {
            vec![
                ProviderEvent::TextDelta(response.message.content),
                ProviderEvent::Done,
            ]
        } else {
            vec![
                ProviderEvent::ToolCalls(response.message.tool_calls),
                ProviderEvent::Done,
            ]
        };
        if let Some(usage) = response.usage {
            events.insert(0, ProviderEvent::Usage(usage));
        }
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }

    async fn fetch_value(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
        mut attempts: Option<&mut PhysicalAttemptControl<'_>>,
    ) -> Result<serde_json::Value, ControlledProviderError> {
        let model = request.model.clone();
        let request_model = model.clone();
        let body = gemini_request::build_body(
            request,
            self.settings.max_output_tokens,
            Some(self.settings.temperature),
        );
        let root = self
            .settings
            .base_url
            .as_deref()
            .unwrap_or("https://generativelanguage.googleapis.com/v1beta")
            .trim_end_matches('/');
        let url = format!("{root}/models/{model}:generateContent");
        if let Some(control) = attempts.as_deref_mut() {
            control.bind_request(crate::PhysicalRequestMetadata::new(
                "gemini",
                &request_model,
                &url,
                self.settings.max_output_tokens,
            )?)?;
        }
        let client = &self.client;
        let key = self.api_key.as_deref();

        let build = || {
            let r = client.post(&url).json(&body);
            if let Some(k) = key {
                r.header("x-goog-api-key", k)
            } else {
                r
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
            super::http::send_stream(
                build,
                &self.settings.retry_delays,
                &cancellation,
                &url,
                self.settings.timeout,
            )
            .await?
        };

        let value: serde_json::Value = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled.into()),
            value = tokio::time::timeout(
                self.settings.timeout,
                super::http::read_json(response, crate::MAX_STREAM_BYTES),
            ) => value
                .map_err(|_| ProviderError::Request(format!(
                    "provider request timed out while reading the response from {url}"
                )))??,
        };

        Ok(value)
    }
}
