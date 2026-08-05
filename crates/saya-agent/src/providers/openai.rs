use super::{
    http::send_stream,
    openai_stream,
    settings::{ProviderSettings, endpoint},
    wire::{messages, tools},
};
use crate::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderStream,
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
        )
        .await?;
        Ok(openai_stream::parse(response, cancellation))
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
    /// Stable key derived from the system prompt so a caching gateway can reuse
    /// the prompt prefix across turns instead of reprocessing it each time.
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
}
impl OpenAiRequest {
    fn from_request(request: ChatRequest, temperature: f32) -> Self {
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
