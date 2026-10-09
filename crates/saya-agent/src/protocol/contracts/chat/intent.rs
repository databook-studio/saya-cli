use serde::{Deserialize, Serialize};

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
/// Each provider translates the variant it honours (`Low`/`Medium`/`High` →
/// `reasoning_effort: "low"|"medium"|"high"` on OpenAI, which **drops
/// `Minimal`** because the `"minimal"` spelling is not universally honoured —
/// see [`crate::OpenAiCompatibleProvider`]; `Minimal` → `think: false` on Ollama, a
/// token budget on Anthropic/Gemini) or drops it; a provider with no equivalent
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
