use serde::{Deserialize, Serialize};

use super::super::ToolDefinition;
use super::{ChatMessage, ReasoningEffort, ResponseFormat};
use crate::protocol::streaming::TokenUsage;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    /// How the caller wants the response shaped. Defaults to [`ResponseFormat::Text`];
    /// the extraction call sets [`ResponseFormat::JsonObject`]. The main agent
    /// loop never sets it (left to `Default`), so a prose answer stays prose.
    #[serde(default)]
    pub response_format: ResponseFormat,
    /// How hard the caller wants the model to think. Defaults to
    /// [`ReasoningEffort::Default`] (send nothing); the extraction call sets
    /// [`ReasoningEffort::Minimal`]. The main agent loop never sets it (left to
    /// `Default`), so the endpoint's own effort configuration wins and the main
    /// loop keeps real reasoning. Separate from `response_format` — the answer's
    /// form is not how hard to think.
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct ChatResponse {
    pub message: ChatMessage,
    /// The chain-of-thought the model produced for this call, or `None` when
    /// the provider reported no reasoning. Lives **here, on `ChatResponse`** —
    /// transport for one call — and never on `ChatMessage`, which is what gets
    /// replayed to the provider as history and what session persistence is
    /// shaped around. That placement is deliberate: with no
    /// reasoning field on `ChatMessage`, a session writer or history builder
    /// has nowhere to copy it, so "reasoning is never persisted" and "reasoning
    /// is never replayed as history" are structural,
    /// not remembered. `None` is the absent case — a provider that omits
    /// chain-of-thought — distinct from `Some(String::new())`, a model that
    /// reasoned and produced nothing; both survive `#[serde(default)]`, so a
    /// response serialized before this field existed (no `reasoning` key)
    /// deserializes to `None`. Capture is unconditional: this is
    /// populated whether or not the user has asked to see thinking — the
    /// `show_thinking` toggle gates *display* only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Token counts the provider reported for this response, or `None` when the
    /// provider reported nothing. `None` is the absent case — distinct from
    /// `Some(TokenUsage::default())`, which would read as "this call cost
    /// nothing" — so a caller cannot mistake a silent provider for a free one
    /// (absent is not zero, mirroring the streaming path, which
    /// models absence by simply not emitting a `Usage` event). The streaming
    /// path's `collect()` threads the last `Usage` event it sees here; Gemini's
    /// non-streaming `complete()` threads the usage it parses directly.
    /// `#[serde(default)]` keeps a serialized response written before this field
    /// existed (no `usage` key) deserializing to `None`.
    #[serde(default)]
    pub usage: Option<TokenUsage>,
}

impl ChatRequest {
    /// A request for `model` carrying `messages`; tools, response format and
    /// reasoning effort stay at their defaults until a caller sets them. This
    /// is the way in from another crate, where the struct is
    /// `#[non_exhaustive]` and a literal will not compile.
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            ..Self::default()
        }
    }

    /// The same request with tool definitions attached, for a call that lets
    /// the model reach for them.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    /// The same request with a response shape requested, for a call site that
    /// needs the answer in a machine-readable form.
    #[must_use]
    pub fn with_response_format(mut self, format: ResponseFormat) -> Self {
        self.response_format = format;
        self
    }

    /// The same request with an effort hint attached. Whether the endpoint
    /// honours it is not knowable from here — see `reasoning_effort`.
    #[must_use]
    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = effort;
        self
    }
}

impl ChatResponse {
    /// A response carrying just the model's message; reasoning and usage stay
    /// absent until a provider reports them. This is the way in from another
    /// crate, where the struct is `#[non_exhaustive]` and a literal will not
    /// compile — a provider implementation sets the extras on the returned
    /// value.
    pub fn new(message: ChatMessage) -> Self {
        Self {
            message,
            ..Self::default()
        }
    }
}
