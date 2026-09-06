use super::{
    http::send_stream,
    openai_stream,
    settings::{ProviderSettings, endpoint},
    wire::{messages, tools},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderStream,
    ReasoningEffort, ResponseFormat,
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
        // No client-wide timeout: streams are bounded per chunk gap instead.
        let client = reqwest::Client::builder()
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
        let body = OpenAiRequest::from_request(request, self.settings.temperature);
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
            &url,
        )
        .await?;
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
    /// own configuration wins; the wire spellings are the provider's own
    /// (`minimal`/`low`/`medium`/`high`).
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
    fn from_request(request: ChatRequest, temperature: f32) -> Self {
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
                ReasoningEffort::Default => None,
                ReasoningEffort::Minimal => Some("minimal"),
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
mod tests {
    use super::*;
    use crate::{ChatMessage, LocalStateEffect, ToolDefinition, ToolEffect};

    fn request_with(format: ResponseFormat, effort: ReasoningEffort) -> ChatRequest {
        ChatRequest {
            model: "test-model".into(),
            messages: vec![ChatMessage::text("user", "extract")],
            tools: vec![ToolDefinition {
                name: "schema_discovery".into(),
                description: "schema".into(),
                read_only: true,
                parameters: serde_json::json!({"type": "object"}),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::None,
                },
            }],
            response_format: format,
            reasoning_effort: effort,
        }
    }

    /// A JSON-mode request carries `response_format: {"type":"json_object"}`
    /// on the OpenAI wire — the spelling the spec verified at 1.7s / 0 reasoning
    /// tokens against the live gateway.
    #[test]
    fn json_object_request_carries_response_format_on_wire() {
        let body = OpenAiRequest::from_request(
            request_with(ResponseFormat::JsonObject, ReasoningEffort::Default),
            0.1,
        );
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            json.contains(r#""response_format":{"type":"json_object"}"#),
            "json_object must appear on the wire: {json}"
        );
    }

    /// A `Text` (default) request omits `response_format`
    /// entirely, so the prose path is byte-identical to a request without JSON mode —
    /// JSON mode is opt-in, never a surprise on the main loop's request.
    #[test]
    fn text_request_omits_response_format_on_wire() {
        let body = OpenAiRequest::from_request(
            request_with(ResponseFormat::Text, ReasoningEffort::Default),
            0.1,
        );
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            !json.contains("response_format"),
            "text request must not carry response_format: {json}"
        );
    }

    /// A `Minimal` effort request carries OpenAI's `reasoning_effort: "minimal"`
    /// spelling on the wire — the direct four-level mapping the provider offers.
    #[test]
    fn minimal_effort_request_carries_reasoning_effort_on_wire() {
        let body = OpenAiRequest::from_request(
            request_with(ResponseFormat::Text, ReasoningEffort::Minimal),
            0.1,
        );
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            json.contains(r#""reasoning_effort":"minimal""#),
            "minimal effort must appear on the wire: {json}"
        );
    }

    /// A `Default` effort request omits `reasoning_effort` entirely, so the
    /// default path sends nothing and the endpoint's own configuration wins —
    /// never a surprise on the main loop's request.
    #[test]
    fn default_effort_request_omits_reasoning_effort_on_wire() {
        let body = OpenAiRequest::from_request(
            request_with(ResponseFormat::Text, ReasoningEffort::Default),
            0.1,
        );
        let json = serde_json::to_string(&body).expect("serializes");
        assert!(
            !json.contains("reasoning_effort"),
            "default effort must not carry reasoning_effort: {json}"
        );
    }

    /// The cache key derives from the session-stable system message, so two
    /// requests that differ only in the last user message share a key and let a
    /// caching gateway reuse the cached prefix — the property the prompt cache
    /// depends on. The per-turn SQL hint rides the user turn (a context block),
    /// so a new question or refined hint never perturbs the key.
    #[test]
    fn prompt_cache_key_is_stable_across_requests_differing_only_in_last_user_message() {
        let system = ChatMessage::text("system", "SAYA system prompt (session-stable)");
        let req_a = OpenAiRequest::from_request(
            ChatRequest::new(
                "test-model",
                vec![system.clone(), ChatMessage::text("user", "first question")],
            ),
            0.1,
        );
        let req_b = OpenAiRequest::from_request(
            ChatRequest::new(
                "test-model",
                vec![
                    system.clone(),
                    ChatMessage::text("user", "follow-up question"),
                ],
            ),
            0.1,
        );
        assert_eq!(
            req_a.prompt_cache_key, req_b.prompt_cache_key,
            "requests differing only in the user turn must share a cache key"
        );
        assert!(
            req_a.prompt_cache_key.is_some(),
            "a request with a system message must produce a cache key"
        );
    }

    /// The cache key is sensitive to the system message: a different
    /// session-stable system prompt yields a different key, so a gateway does
    /// not cross-share prefixes between sessions with different connections or
    /// memory mode.
    #[test]
    fn prompt_cache_key_changes_when_the_system_message_changes() {
        let a = OpenAiRequest::from_request(
            ChatRequest::new(
                "test-model",
                vec![ChatMessage::text("system", "system prompt A")],
            ),
            0.1,
        );
        let b = OpenAiRequest::from_request(
            ChatRequest::new(
                "test-model",
                vec![ChatMessage::text("system", "system prompt B")],
            ),
            0.1,
        );
        assert_ne!(
            a.prompt_cache_key, b.prompt_cache_key,
            "different system messages must yield different cache keys"
        );
    }

    /// A request with no system message carries no cache key: there is no
    /// session-stable prefix for a gateway to cache.
    #[test]
    fn prompt_cache_key_is_none_without_a_system_message() {
        let body = OpenAiRequest::from_request(
            ChatRequest::new("test-model", vec![ChatMessage::text("user", "hi")]),
            0.1,
        );
        assert!(body.prompt_cache_key.is_none());
    }
}
