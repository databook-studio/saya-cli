use super::super::Admission;
use crate::{AgentError, CancellationReason, CancellationToken, EffectOwnerFailure};

pub(super) fn cancelled(cancellation: &CancellationToken) -> AgentError {
    Admission::cancellation_error(cancellation.reason(), true).unwrap_or(AgentError::Cancelled)
}

pub(super) fn owner_failure(
    cancellation: &CancellationToken,
    error: EffectOwnerFailure,
) -> AgentError {
    cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
    AgentError::EffectOwnerFailure(error)
}

pub(super) fn contract_failure(cancellation: &CancellationToken) -> AgentError {
    owner_failure(cancellation, EffectOwnerFailure::contract_violation())
}
