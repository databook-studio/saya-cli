//! Reasoning replay bookkeeping.
//!
//! The provider captures one turn's chain-of-thought; the loop keeps it on the
//! assistant message it came from so every later request replays it. The copy
//! is **append-only**: a new reply adds its own reasoning and never clears an
//! earlier message's, because a prompt cache is a prefix cache and clearing an
//! earlier copy rewrites the request prefix for every token after it. Each copy
//! is capped by [`crate::ChatMessage::with_reasoning`] and never persisted
//! (`#[serde(skip)]` on the field); the number of copies is the number of
//! assistant turns, itself bounded by the run's turn and request budgets.

use super::attempt::ReasoningTrace;
use crate::ChatMessage;

/// Attach the turn's reasoning and, when the route issued one, its signature
/// to the assistant message that produced them, leaving every earlier
/// message's copy in place. Returns the assistant message unchanged when the
/// provider reported nothing.
pub(super) fn attach(assistant: ChatMessage, trace: Option<ReasoningTrace>) -> ChatMessage {
    match trace {
        Some(trace) => assistant.with_reasoning_trace(trace.text, trace.signature),
        None => assistant,
    }
}

#[cfg(test)]
#[path = "reasoning_replay_tests.rs"]
mod tests;
