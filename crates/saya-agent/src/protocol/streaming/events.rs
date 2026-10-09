use super::TokenUsage;
use crate::{ProviderError, ToolCall};
use futures_util::Stream;
use std::pin::Pin;

/// Ceiling on one provider response's accumulated bytes. A misbehaving or
/// hostile endpoint must not be able to stream unbounded data into memory.
pub const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;
/// Maximum distinct tool calls accepted from one assistant response.
pub const MAX_TOOL_CALLS_PER_RESPONSE: usize = 256;

/// One increment from a provider stream.
///
/// `#[non_exhaustive]` is here for semver headroom: `saya-agent` is a published
/// crate, and without it every new variant is a breaking change for anyone
/// matching on this enum.
///
/// It is deliberately *not* a defence against the "unhandled variant becomes
/// `Not implemented: …` under a correct answer" regression that has happened
/// three times here (most recently `a4fe388`). The attribute forces external
/// matches to carry a `_` arm, and a `_` arm is precisely what swallows a new
/// variant — it mandates the catch-all rather than preventing it. That
/// regression was in `AgentEvent` handling in `saya-cli`, and the fix for it is
/// `stream_render.rs` returning `Option`, not anything on this type. Nothing
/// outside this crate matches `ProviderEvent` today.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderEvent {
    TextDelta(String),
    /// Chain-of-thought from a reasoning model. Captured unconditionally
    /// (parse whether or not the user has asked to see it — the
    /// `show_thinking` toggle gates display only, not a precondition for capture) and
    /// accumulated under the same `MAX_STREAM_BYTES` bound as `TextDelta`, so a
    /// hostile endpoint cannot stream unbounded "thinking" into memory. Never
    /// reaches a session file or the provider as history: reasoning lives on
    /// `ChatResponse` (transport for one call), never on `ChatMessage` (what
    /// gets replayed and persisted), so there is no field to copy it through.
    ReasoningDelta(String),
    ToolCalls(Vec<ToolCall>),
    /// The provider's cumulative token counts so far for this response.
    Usage(TokenUsage),
    Done,
}
pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send>>;
