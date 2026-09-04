//! Chat conversation contracts: the agent request, the messages replayed to a
//! provider, and the provider call's request/response and intent options.

use serde::{Deserialize, Serialize};

use super::ToolDefinition;
use crate::protocol::streaming::TokenUsage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub prompt: String,
    pub profile_names: Vec<String>,
    pub model: String,
    /// Optional extra system context appended to the base SAYA system prompt (e.g. available database connections).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub history: Vec<ChatMessage>,
    /// Untrusted, labelled database context rendered into the user turn — never the
    /// system message. `#[serde(default)]` keeps old serialized requests deserializable;
    /// `skip_serializing_if` keeps an empty vector off the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_blocks: Vec<ContextBlock>,
}

/// A labelled, untrusted chunk of database context (a contract, a schema note, a
/// comment) that reaches the model as quoted data inside the user turn, never as
/// policy in the system message. Nothing populates this in Phase 2a; Phase 2b wires
/// recall in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBlock {
    /// Short machine-ish label for the block's source, e.g. "database-contracts".
    pub label: String,
    /// The block's content. Untrusted.
    pub body: String,
    /// True when the source had more to give than the caller's budget allowed.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn text(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolMetadata {
    pub name: String,
    pub status: String,
}

/// The shape the caller wants the response in. Provider-neutral intent —
/// **not** an OpenAI wire spelling — so `saya-agent` stays provider-agnostic.
/// Each provider translates the variant it honours (`JsonObject` →
/// `response_format: {"type":"json_object"}` on OpenAI, `format: "json"` on
/// Ollama) or drops it; a provider with no equivalent degrades to today's
/// behaviour (the prompt already asks for JSON, `strip_markdown_fences` already
/// handles fences). `Text` is the default so the main agent loop — which never
/// sets this — keeps answering in prose (JSON mode is for the
/// extraction call only).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Free-form prose. The default; the main loop's request.
    #[default]
    Text,
    /// The response must be a single JSON object. The extraction call sets this
    /// so a reasoning model does not spend tokens on chain-of-thought we discard.
    JsonObject,
}

/// How hard the caller wants the model to think. Provider-neutral intent —
/// **not** an OpenAI wire spelling — so `saya-agent` stays provider-agnostic.
/// Each provider translates the variant it honours (`Minimal` →
/// `reasoning_effort: "minimal"` on OpenAI, `think: false` on Ollama, a token
/// budget on Anthropic/Gemini) or drops it; a provider with no equivalent
/// degrades to today's behaviour, never to an error. `Default` is the default
/// and means *send nothing*: the main agent loop — which never sets this —
/// leaves effort to the endpoint, so a self-hosted gateway operator's own
/// configuration wins (the main loop keeps real reasoning; only
/// mechanical call sites request less). Whether the model *complied* is only
/// knowable from the reported reasoning tokens, so saya reports what it asked
/// for, never that an effort was applied.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Send nothing; let the endpoint's own configuration decide. The default;
    /// the main loop's request.
    #[default]
    Default,
    /// Ask for the least thinking the provider offers.
    Minimal,
    /// Ask for less thinking than the endpoint would do unasked.
    Low,
    /// A middle effort.
    Medium,
    /// Ask for the most thinking the provider offers.
    High,
}

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
