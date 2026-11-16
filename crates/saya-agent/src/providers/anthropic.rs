use super::{
    anthropic_request, anthropic_stream, attribution,
    http::{RetryPolicy, send_stream},
    settings::{ProviderSettings, endpoint},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ControlledProviderError,
    PhysicalAttemptControl, ProviderError, ProviderStream,
};
use async_trait::async_trait;

/// The ceiling this route sends when the operator configured none. Anthropic
/// requires `max_tokens` on every request, so the limit cannot be omitted the
/// way it can on the OpenAI-compatible, Gemini and Ollama wires.
const DEFAULT_MAX_TOKENS: u32 = 32_000;

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

    /// A provider over a caller-built client, so a test can resolve
    /// `openrouter.ai` to its own loopback server without touching DNS.
    #[cfg(test)]
    pub(crate) fn with_client(
        settings: ProviderSettings,
        api_key: Option<&str>,
        client: reqwest::Client,
    ) -> Result<Self, ProviderError> {
        settings.validate_retries()?;
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
    fn retry_delays(&self) -> Vec<std::time::Duration> {
        self.settings.retry_delays.clone()
    }
    fn max_output_tokens(&self) -> Option<u32> {
        self.settings.max_output_tokens
    }
    fn configured_reasoning_effort(&self) -> Option<crate::ReasoningEffort> {
        self.settings.reasoning_effort
    }
    fn request_timeout(&self) -> std::time::Duration {
        self.settings.request_timeout
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
        // The caller's effort when it stated one, the operator's configured
        // effort otherwise. Resolved before anything reads it, so the beta
        // header below and the body's thinking block always agree.
        let mut request = request;
        // An effort the operator pinned in configuration is "stated" even when
        // the loop repeats it on every request; only an unconfigured policy
        // choice may use the provider's adaptive control for `low`.
        let source = if self.settings.reasoning_effort.is_none() {
            anthropic_request::EffortSource::Policy
        } else {
            anthropic_request::EffortSource::Stated
        };
        request.reasoning_effort = self
            .settings
            .effective_reasoning_effort(request.reasoning_effort);
        let openrouter = self.settings.is_openrouter();
        let adaptive =
            anthropic_request::uses_adaptive(request.reasoning_effort, source, openrouter);
        let max_output_tokens = self
            .settings
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_TOKENS);
        let body = anthropic_request::build_body(
            request,
            max_output_tokens,
            Some(self.settings.temperature),
            source,
            openrouter,
        );
        let url = endpoint(
            self.settings.base_url.as_deref(),
            "https://api.anthropic.com/v1",
            "messages",
        );
        if let Some(control) = attempts.as_deref_mut() {
            let streamed = body.get("stream").and_then(serde_json::Value::as_bool);
            control.bind_request(
                crate::PhysicalRequestMetadata::new(
                    "anthropic",
                    &request_model,
                    &url,
                    Some(max_output_tokens),
                )?
                .with_thinking(thinking_telemetry(&body), streamed),
            )?;
        }
        let client = &self.client;
        let key = self.api_key.as_deref();
        let build = || {
            let request = attribution::apply(client.post(&url).json(&body), &self.settings);
            let request = request.header("anthropic-version", "2023-06-01");
            let request = if adaptive {
                request.header("anthropic-beta", "effort-2025-11-24")
            } else {
                request
            };
            if let Some(k) = key {
                request.header("x-api-key", k)
            } else {
                request
            }
        };
        let retry = RetryPolicy::new(&self.settings.retry_delays);
        // Establishment may not outlast the total request deadline either.
        let establishment_timeout = self.settings.timeout.min(self.settings.request_timeout);
        let response = if let Some(control) = attempts {
            super::http::send_stream_controlled(
                build,
                retry,
                &cancellation,
                &url,
                establishment_timeout,
                control,
            )
            .await?
        } else {
            send_stream(build, retry, &cancellation, &url, establishment_timeout).await?
        };
        Ok(anthropic_stream::parse(
            response,
            cancellation,
            self.settings.idle_timeout,
            self.settings.request_timeout,
            self.settings.stall_timeout,
        ))
    }
}

/// The thinking control the finalized body carries, read back from the body
/// itself rather than inferred from configuration. A mode without its matching
/// control is not a measurement and is reported as absent.
fn thinking_telemetry(body: &serde_json::Value) -> Option<saya_types::ThinkingSpanData> {
    use saya_types::{ThinkingEffort, ThinkingMode, ThinkingSpanData};
    let thinking = body.get("thinking")?;
    let mode = match thinking.get("type").and_then(serde_json::Value::as_str)? {
        "adaptive" => ThinkingMode::Adaptive,
        "enabled" => ThinkingMode::Enabled,
        _ => return None,
    };
    let budget_tokens = thinking
        .get("budget_tokens")
        .and_then(serde_json::Value::as_u64)
        .and_then(|budget| u32::try_from(budget).ok());
    let effort = body
        .get("output_config")
        .and_then(|config| config.get("effort"))
        .and_then(serde_json::Value::as_str)
        .and_then(|effort| match effort {
            "low" => Some(ThinkingEffort::Low),
            "medium" => Some(ThinkingEffort::Medium),
            "high" => Some(ThinkingEffort::High),
            _ => None,
        });
    match mode {
        ThinkingMode::Adaptive if effort.is_some() && budget_tokens.is_none() => {
            Some(ThinkingSpanData {
                mode,
                budget_tokens,
                effort,
            })
        }
        ThinkingMode::Enabled if budget_tokens.is_some() && effort.is_none() => {
            Some(ThinkingSpanData {
                mode,
                budget_tokens,
                effort,
            })
        }
        _ => None,
    }
}
