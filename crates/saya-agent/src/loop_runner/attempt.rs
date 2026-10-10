//! Internal outcomes for one provider receive attempt.

use crate::{
    AgentError, ChatMessage, EffectOwnerFailure, ProviderError, ProviderRecoveryReason, TokenUsage,
};

/// The three existing 250/500/1000 ms retry delays.
pub(super) const RETRY_LIMIT: u8 = 3;
/// Initial attempt plus the three fixed retry delays.
pub(super) const MAX_ATTEMPTS: u8 = RETRY_LIMIT + 1;

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

pub(super) enum AttemptOutcome {
    Success {
        message: ChatMessage,
        usage: Option<TokenUsage>,
        reasoning: Option<String>,
    },
    Failed {
        error: AttemptError,
        usage: Option<TokenUsage>,
    },
}
