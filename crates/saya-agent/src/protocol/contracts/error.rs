//! Provider errors that surface from the agent protocol.
//!
//! Tool-call failures live in the sibling [`tool_error`] module and are
//! re-exported here so every protocol error keeps one import path.

use crate::protocol::streaming::TokenUsage;

pub use super::tool_error::ToolError;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderError {
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned an invalid response")]
    InvalidResponse,
    /// The response stream ended before its terminator: a cut connection or an
    /// early close rather than malformed bytes. Payload-free on purpose — what
    /// the stream had delivered is discarded, so the error carries no
    /// provider text.
    #[error("provider stream ended before its terminator")]
    StreamEnded,
    /// The transport stayed alive but the model produced no text, reasoning,
    /// or tool-call delta for the provider's stall window. Distinct from the
    /// idle timeout, which a gateway can defeat with keep-alives. Retryable:
    /// the identical request can succeed when the model recovers.
    #[error("provider stream stalled with no content")]
    StreamStalled,
    /// The response stream was cut mid-flow: the provider had already
    /// delivered content bytes, then the stream ended before its terminator.
    /// Distinct from [`ProviderError::StreamEnded`], which is what a stream
    /// that delivered nothing reports. A cut that delivered content is
    /// deterministic — a gateway response time limit cuts the identical
    /// request at the same point — so the identical request must never be
    /// re-sent; the loop spends one bounded continuation instead. Payload-free
    /// of provider text: what the stream delivered is discarded, so the error
    /// carries only the cut attempt's own clock and the token count estimated
    /// from the bytes actually received (a cut reports no usage).
    #[error(
        "provider stream was cut before its terminator after {elapsed_secs}s (~{estimated_tokens} tokens received)"
    )]
    StreamCut {
        /// Whole seconds the stream ran, measured from the stream's own clock.
        elapsed_secs: u32,
        /// Tokens estimated from the content bytes the stream delivered
        /// before the cut (reasoning + text + tool-call argument bytes).
        estimated_tokens: u32,
    },
    #[error("provider tool calls exceeded collection limit")]
    ToolCollectionLimit,
    #[error("provider stream exceeded item limit")]
    StreamItemLimit {
        /// Latest usage already reported before the attempt crossed its item cap.
        usage: Option<Box<TokenUsage>>,
    },
    /// The model hit its per-response output-token limit mid-answer. Distinct
    /// from a transport failure: re-sending the identical request fails
    /// identically, so the loop must not retry it. Carries the redacted
    /// partial output the wire had already emitted. Construct via
    /// [`ProviderError::output_truncated`], which redacts both partials —
    /// a struct literal would bypass that.
    #[error("provider response truncated: the model hit its output-token limit")]
    OutputTruncated {
        /// Text the wire emitted before the cap, redacted at construction.
        partial_text: String,
        /// Raw tool-argument fragments assembled before the cap, one per
        /// partial call, redacted at construction.
        partial_tool_json: Vec<String>,
    },
    #[error("provider is not configured: {0}")]
    Configuration(String),
    #[error("provider stream was cancelled")]
    Cancelled,
}

impl ProviderError {
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    /// Builds the truncation signal with both partials passed through the
    /// same redaction as every other model output, so a credential-shaped
    /// substring in a cut-off answer cannot leak through the error path.
    pub fn output_truncated(partial_text: String, partial_tool_json: Vec<String>) -> Self {
        Self::OutputTruncated {
            partial_text: saya_types::redact(&partial_text),
            partial_tool_json: partial_tool_json
                .iter()
                .map(|json| saya_types::redact(json))
                .collect(),
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
