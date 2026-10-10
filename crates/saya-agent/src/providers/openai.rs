use super::{
    http::send_stream,
    openai_stream,
    settings::{ProviderSettings, endpoint},
    wire::{messages, tools},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ControlledProviderError,
    PhysicalAttemptControl, ProviderError, ProviderStream, ReasoningEffort, ResponseFormat,
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
        settings.validate_retries()?;
        // Establishment is bounded per request; stream bodies are bounded by
        // the parser's per-chunk idle timeout instead of a total cap.
        // Redirects are refused outright: `base_url` is user-configurable, so
        // a misconfigured or hostile endpoint could answer 307 and have the
        // default policy replay the POST — prompt plus database-derived
        // context — to another host. (`Authorization` would be stripped on
        // the cross-host hop; the prompt body would not.)
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

impl OpenAiCompatibleProvider {
    async fn stream_inner(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
        attempts: Option<&mut PhysicalAttemptControl<'_>>,
    ) -> Result<ProviderStream, ControlledProviderError> {
        let body = OpenAiRequest::from_request(
            request,
            self.settings.temperature,
            self.settings.max_output_tokens,
        );
        let url = endpoint(
            self.settings.base_url.as_deref(),
            "https://api.openai.com/v1",
            "chat/completions",
        );
        let client = &self.client;
        let key = self.api_key.as_deref();
        let build = || {
            let request = client.post(&url).json(&body);
            if let Some(value) = key {
                request.bearer_auth(value)
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
        Ok(openai_stream::parse(
            response,
            cancellation,
            self.settings.idle_timeout,
        ))
    }
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<super::wire::WireMessage>,
    tools: Vec<super::wire::WireTool>,
    stream: bool,
    /// Sampling temperature (configurable via `[ai].temperature`, default 0.1).
    /// Lower keeps answers concise and deterministic (fewer tokens/loops).
    temperature: f32,
    /// Per-response output-token ceiling. Sent as `max_completion_tokens`:
    /// `max_tokens` is deprecated and rejected by newer reasoning models,
    /// while `max_completion_tokens` covers visible output plus reasoning
    /// tokens on every model.
    max_completion_tokens: u32,
    /// Stable key derived from the **system message** so a caching gateway can
    /// reuse the prompt prefix across turns instead of reprocessing it each time.
    ///
    /// The key is the FNV-1a hash of the system message content. The system
    /// message is **session-stable by construction**: it is the fixed SAYA system
    /// prompt plus the assembled extra (connection descriptions, memory
    /// briefing, engine/dialect guidance, working guidance, answer contract) —
    /// all of which depend only on the session's connections and memory mode,
    /// never on a single turn. The per-turn last-SQL hint lives on the **user**
    /// turn (as a context block), so it never perturbs this key. Two requests
    /// that differ only in the user turn — a new question, a refined SQL hint —
    /// therefore share a key and let the gateway reuse the cached system prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    /// Ask the gateway for token counts on a trailing usage-only chunk.
    stream_options: StreamOptions,
    /// The OpenAI `response_format` spelling of [`ChatRequest::response_format`].
    /// Only present when the caller asked for JSON — omitted for `Text` so the
    /// default prose path is byte-identical to before this field existed
    /// (JSON mode is opt-in, extraction-call only).
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormatWire>,
    /// The OpenAI `reasoning_effort` spelling of [`ChatRequest::reasoning_effort`].
    /// Omitted for `Default` so the default path sends nothing and the endpoint's
    /// own configuration wins. Omitted for `Minimal` too: the `"minimal"` spelling
    /// is the one variant outside the `{low, medium, high}` set that the
    /// OpenAI-family wire spells, and it is not universally honoured — a
    /// Fireworks-backed gateway rejects `reasoning_effort: "minimal"` with HTTP
    /// 400 (databook-studio/saya-cli#56), which fails extraction every turn.
    /// The `ReasoningEffort` contract is that a provider which cannot honour a
    /// variant **drops it, never errors**, so `Minimal` is dropped on this wire
    /// and the endpoint's own (lowest) effort configuration wins. The
    /// `ChatRequest` still carries `Minimal`, so providers that do translate it
    /// (Ollama's `think: false`, Anthropic/Gemini's token budget) are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
}

/// The OpenAI wire shape for `response_format`. Only `json_object` is emitted;
/// `text` is the gateway default, so it is never sent.
#[derive(Serialize)]
struct ResponseFormatWire {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

impl OpenAiRequest {
    fn from_request(request: ChatRequest, temperature: f32, max_output_tokens: u32) -> Self {
        // The cache key derives from the system message — the session-stable
        // prefix a gateway caches. See `prompt_cache_key` for why the system
        // message is stable across turns (the per-turn SQL hint rides the user
        // turn, never the system message).
        let prompt_cache_key = request
            .messages
            .iter()
            .find(|message| message.role == "system")
            .map(|message| fnv1a_hex(&message.content));
        Self {
            model: request.model,
            messages: messages(request.messages),
            tools: tools(request.tools),
            stream: true,
            temperature,
            max_completion_tokens: max_output_tokens,
            prompt_cache_key,
            stream_options: StreamOptions {
                include_usage: true,
            },
            response_format: match request.response_format {
                ResponseFormat::JsonObject => Some(ResponseFormatWire {
                    kind: "json_object",
                }),
                ResponseFormat::Text => None,
            },
            reasoning_effort: match request.reasoning_effort {
                ReasoningEffort::Default | ReasoningEffort::Minimal => None,
                ReasoningEffort::Low => Some("low"),
                ReasoningEffort::Medium => Some("medium"),
                ReasoningEffort::High => Some("high"),
            },
        }
    }
}

/// Deterministic FNV-1a hash (stable across processes, unlike the std hasher),
/// used to derive a stable prompt-cache key from the system prompt.
fn fnv1a_hex(input: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
#[path = "openai/tests.rs"]
mod tests;
