use super::{gemini_request, gemini_response, settings::ProviderSettings};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderEvent,
    ProviderStream,
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
        // Redirects are refused outright: `base_url` is user-configurable, so
        // a misconfigured or hostile endpoint could answer 307 and have the
        // default policy replay the POST to another host — prompt plus
        // database-derived context in the body, and the `x-goog-api-key`
        // header untouched by reqwest's cross-host strip (which removes only
        // `Authorization` and cookie headers).
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
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
        self.complete_with_cancellation(request, CancellationToken::new())
            .await
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let response = self
            .complete_with_cancellation(request, cancellation)
            .await?;
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
}

impl GeminiProvider {
    async fn complete_with_cancellation(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ChatResponse, ProviderError> {
        let model = request.model.clone();
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
            &url,
            self.settings.timeout,
        )
        .await?;

        let value: serde_json::Value = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            value = tokio::time::timeout(
                self.settings.timeout,
                super::http::read_json(response, crate::MAX_STREAM_BYTES),
            ) => value
                .map_err(|_| ProviderError::Request(format!(
                    "provider request timed out while reading the response from {url}"
                )))??,
        };

        gemini_response::parse(value)
    }
}
