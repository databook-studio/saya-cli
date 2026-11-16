//! Internal outcomes for one provider receive attempt.

use crate::{
    AgentError, ChatMessage, EffectOwnerFailure, ProviderError, ProviderRecoveryReason, TokenUsage,
};

/// The most turn-level retries any retry schedule can ask for: one per entry
/// of the eight-entry default schedule.
pub(super) const RETRY_LIMIT: u8 = 8;

#[derive(Debug)]
pub(super) struct ReceiveFailure {
    pub(super) error: AgentError,
    pub(super) usage: TokenUsage,
}

pub(super) enum AttemptError {
    Cancelled,
    AdmissionDenied,
    OwnerFailure(EffectOwnerFailure),
    UnsupportedPhysicalCoverage,
    Retryable {
        error: ProviderError,
        reason: ProviderRecoveryReason,
    },
    Terminal {
        error: ProviderError,
        reason: ProviderRecoveryReason,
    },
}

impl AttemptError {
    pub(super) fn error(self) -> AgentError {
        match self {
            Self::Cancelled => AgentError::Cancelled,
            Self::AdmissionDenied => AgentError::EffectAdmissionDenied,
            Self::OwnerFailure(error) => AgentError::EffectOwnerFailure(error),
            Self::UnsupportedPhysicalCoverage => AgentError::UnsupportedPhysicalCoverage,
            Self::Retryable { error, .. } | Self::Terminal { error, .. } => {
                AgentError::Provider(error)
            }
        }
    }

    pub(super) fn reason(&self) -> ProviderRecoveryReason {
        match self {
            Self::Cancelled => ProviderRecoveryReason::Cancelled,
            Self::AdmissionDenied | Self::OwnerFailure(_) | Self::UnsupportedPhysicalCoverage => {
                ProviderRecoveryReason::ProviderFailure
            }
            Self::Retryable { reason, .. } | Self::Terminal { reason, .. } => *reason,
        }
    }

    pub(super) fn into_control_failure(self) -> Result<Self, AgentError> {
        match self {
            Self::AdmissionDenied => Err(AgentError::EffectAdmissionDenied),
            Self::OwnerFailure(error) => Err(AgentError::EffectOwnerFailure(error)),
            Self::UnsupportedPhysicalCoverage => Err(AgentError::UnsupportedPhysicalCoverage),
            error => Ok(error),
        }
    }
}

/// One turn's chain-of-thought and, when the route issued one, the provider's
/// signature over it. Both travel together or the text replays alone.
#[derive(Debug)]
pub(super) struct ReasoningTrace {
    pub(super) text: Option<String>,
    pub(super) signature: Option<String>,
}

impl ReasoningTrace {
    pub(super) fn new(text: Option<String>, signature: Option<String>) -> Option<Self> {
        (text.is_some() || signature.is_some()).then_some(Self { text, signature })
    }
}

pub(super) enum AttemptOutcome {
    Success {
        message: ChatMessage,
        usage: Option<TokenUsage>,
        reasoning: Option<ReasoningTrace>,
    },
    Failed {
        error: AttemptError,
        usage: Option<TokenUsage>,
    },
}
