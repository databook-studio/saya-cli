use super::super::admission::coding;
use crate::{
    AgentError, CancellationToken, EffectAdmission, EffectCallKind, EffectOutcome, EffectPermit,
    PhysicalAttemptControl, TokenUsage,
};

pub(super) fn control<'a>(
    required: bool,
    owner: Option<&'a dyn EffectAdmission>,
    parent: Option<EffectPermit>,
    call_kind: EffectCallKind,
    cancellation: &'a CancellationToken,
) -> Result<Option<PhysicalAttemptControl<'a>>, AgentError> {
    match (required, owner, parent) {
        (true, Some(owner), Some(parent)) => Ok(Some(PhysicalAttemptControl::new(
            owner,
            cancellation,
            parent,
            call_kind,
        ))),
        (true, _, _) => Err(AgentError::UnsupportedPhysicalCoverage),
        (false, _, _) => Ok(None),
    }
}

pub(super) async fn settle_request(
    owner: Option<&dyn EffectAdmission>,
    permit: Option<EffectPermit>,
    response_received: bool,
    physical_attempts: &Option<PhysicalAttemptControl<'_>>,
    reported_usage: Option<TokenUsage>,
    mut usage_complete: bool,
    cancellation: &CancellationToken,
) -> Result<(), AgentError> {
    let outcome = if response_received {
        EffectOutcome::Succeeded
    } else if physical_attempts
        .as_ref()
        .is_some_and(|attempts| !attempts.dispatched_any())
    {
        EffectOutcome::NotStarted
    } else {
        EffectOutcome::Unknown
    };
    if let Some(attempts) = physical_attempts {
        usage_complete &= attempts.usage_complete();
    }
    if outcome == EffectOutcome::NotStarted {
        usage_complete = false;
    }
    coding::settle_model_request(
        owner,
        permit,
        outcome,
        reported_usage,
        usage_complete,
        cancellation,
    )
    .await
}
