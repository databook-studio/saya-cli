//! Internal outcomes for one provider receive attempt.

use crate::{AgentError, ChatMessage, ProviderError, ProviderRecoveryReason, TokenUsage};

/// The three existing 250/500/1000 ms retry delays.
pub(super) const RETRY_LIMIT: u8 = 3;

#[derive(Debug)]
pub(super) struct ReceiveFailure {
    pub(super) error: AgentError,
    pub(super) usage: TokenUsage,
}

pub(super) enum AttemptError {
    Cancelled,
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
            Self::Retryable { error, .. } | Self::Terminal { error, .. } => {
                AgentError::Provider(error)
            }
        }
    }

    pub(super) fn reason(&self) -> ProviderRecoveryReason {
        match self {
            Self::Cancelled => ProviderRecoveryReason::Cancelled,
            Self::Retryable { reason, .. } | Self::Terminal { reason, .. } => *reason,
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
