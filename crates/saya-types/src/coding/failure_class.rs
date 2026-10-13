//! A closed, payload-free reason a coding child failed. Never provider text: only which kind of
//! stop it was, so a failed job can be explained from its export without leaking detail.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingFailureClass {
    /// The model spent its output-token limit before answering (a cut answer).
    OutputLimit,
    /// The provider call failed (transport, timeout, malformed or refused response).
    ProviderError,
    /// The agent loop reached one of its own limits.
    LimitReached,
    /// The model produced a tool call or history the loop could not use.
    InvalidToolCall,
    /// The job's shared budget or effect owner refused the next step.
    AdmissionDenied,
    /// The provider cannot supply the accounting the job requires.
    Unsupported,
    /// The child could not be set up or its channel identity did not match.
    SetupFailed,
    /// The child was cancelled by something other than the user.
    Cancelled,
    /// Anything else, including owner failures.
    Internal,
}

impl CodingFailureClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OutputLimit => "output_limit",
            Self::ProviderError => "provider_error",
            Self::LimitReached => "limit_reached",
            Self::InvalidToolCall => "invalid_tool_call",
            Self::AdmissionDenied => "admission_denied",
            Self::Unsupported => "unsupported",
            Self::SetupFailed => "setup_failed",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal",
        }
    }
}
