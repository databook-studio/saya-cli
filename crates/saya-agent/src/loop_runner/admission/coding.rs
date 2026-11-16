mod error;

use error::{cancelled, contract_failure, owner_failure};
use std::collections::HashSet;

use crate::{
    AgentError, CancellationToken, EffectAdmission, EffectAdmissionError, EffectOutcome,
    EffectOwnerFailure, EffectPermit, EffectSettlement, TokenUsage, ToolEffect,
};

use super::Admission;

pub(in crate::loop_runner) async fn admit_model_request(
    owner: Option<&dyn EffectAdmission>,
    cancellation: &CancellationToken,
) -> Result<Option<EffectPermit>, AgentError> {
    let Some(owner) = owner else {
        return Ok(None);
    };
    let permit = match owner.admit_model_request().await {
        Ok(permit) => permit,
        Err(EffectAdmissionError::Denied) => return Err(AgentError::EffectAdmissionDenied),
        Err(EffectAdmissionError::OwnerFailure(error)) => {
            return Err(owner_failure(cancellation, error));
        }
        Err(_) => return Err(contract_failure(cancellation)),
    };
    if cancellation.is_cancelled() {
        let error = cancelled(cancellation);
        settle_model_request(
            Some(owner),
            Some(permit),
            EffectOutcome::NotStarted,
            None,
            false,
            cancellation,
        )
        .await?;
        return Err(error);
    }
    Ok(Some(permit))
}

pub(in crate::loop_runner) async fn admit_tool_batch(
    owner: Option<&dyn EffectAdmission>,
    effects: &[ToolEffect],
    cancellation: &CancellationToken,
) -> Result<Option<Vec<EffectPermit>>, AgentError> {
    let Some(owner) = owner else {
        return Ok(None);
    };
    let permits = match owner.admit_tool_batch(effects).await {
        Ok(permits) => permits,
        Err(EffectAdmissionError::Denied) => return Err(AgentError::EffectAdmissionDenied),
        Err(EffectAdmissionError::OwnerFailure(error)) => {
            return Err(owner_failure(cancellation, error));
        }
        Err(_) => return Err(contract_failure(cancellation)),
    };
    let mut seen = HashSet::with_capacity(permits.len());
    let valid =
        permits.len() == effects.len() && permits.iter().all(|permit| seen.insert(permit.get()));
    if !valid {
        settle_unstarted_permits(owner, &permits, cancellation).await?;
        return Err(contract_failure(cancellation));
    }
    if cancellation.is_cancelled() {
        let error = cancelled(cancellation);
        settle_unstarted_permits(owner, &permits, cancellation).await?;
        return Err(error);
    }
    Ok(Some(permits))
}

pub(in crate::loop_runner) async fn admit_tool(
    owner: Option<&dyn EffectAdmission>,
    effect: ToolEffect,
    cancellation: &CancellationToken,
) -> Result<Option<EffectPermit>, AgentError> {
    let Some(mut permits) =
        admit_tool_batch(owner, std::slice::from_ref(&effect), cancellation).await?
    else {
        return Ok(None);
    };
    permits
        .pop()
        .map(Some)
        .ok_or_else(|| contract_failure(cancellation))
}

pub(in crate::loop_runner) async fn check_tool_dispatch(
    owner: Option<&dyn EffectAdmission>,
    permit: Option<EffectPermit>,
    cancellation: &CancellationToken,
) -> Result<(), AgentError> {
    let Some(error) = Admission::cancellation_error(cancellation.reason(), owner.is_some()) else {
        return Ok(());
    };
    settle_tool(owner, permit, EffectOutcome::NotStarted, cancellation).await?;
    Err(error)
}

pub(in crate::loop_runner) async fn settle_model_request(
    owner: Option<&dyn EffectAdmission>,
    permit: Option<EffectPermit>,
    outcome: EffectOutcome,
    reported_usage: Option<TokenUsage>,
    usage_complete: bool,
    cancellation: &CancellationToken,
) -> Result<(), AgentError> {
    let (Some(owner), Some(permit)) = (owner, permit) else {
        return if owner.is_none() && permit.is_none() {
            Ok(())
        } else {
            Err(contract_failure(cancellation))
        };
    };
    owner
        .settle(
            permit,
            EffectSettlement::ModelRequest {
                outcome,
                reported_usage,
                usage_complete,
            },
        )
        .await
        .map_err(|error| owner_failure(cancellation, error))
}

pub(in crate::loop_runner) async fn settle_tool(
    owner: Option<&dyn EffectAdmission>,
    permit: Option<EffectPermit>,
    outcome: EffectOutcome,
    cancellation: &CancellationToken,
) -> Result<(), AgentError> {
    let (Some(owner), Some(permit)) = (owner, permit) else {
        return if owner.is_none() && permit.is_none() {
            Ok(())
        } else {
            Err(contract_failure(cancellation))
        };
    };
    owner
        .settle(permit, EffectSettlement::Tool { outcome })
        .await
        .map_err(|error| owner_failure(cancellation, error))
}

pub(in crate::loop_runner) async fn settle_tool_result(
    owner: Option<&dyn EffectAdmission>,
    permit: Option<EffectPermit>,
    succeeded: bool,
    cancellation: &CancellationToken,
) -> Result<(), AgentError> {
    settle_tool(
        owner,
        permit,
        if succeeded {
            EffectOutcome::Succeeded
        } else {
            EffectOutcome::Unknown
        },
        cancellation,
    )
    .await
}

pub(in crate::loop_runner) async fn settle_tool_batch(
    owner: Option<&dyn EffectAdmission>,
    permits: Option<&[EffectPermit]>,
    outcomes: &[EffectOutcome],
    cancellation: &CancellationToken,
) -> Result<(), AgentError> {
    let (Some(owner), Some(permits)) = (owner, permits) else {
        return if owner.is_none() && permits.is_none() {
            Ok(())
        } else {
            Err(contract_failure(cancellation))
        };
    };
    if permits.len() != outcomes.len() {
        let mut first_failure = None;
        for &permit in permits {
            if let Err(error) = owner
                .settle(
                    permit,
                    EffectSettlement::Tool {
                        outcome: EffectOutcome::Unknown,
                    },
                )
                .await
                && first_failure.is_none()
            {
                first_failure = Some(error);
            }
        }
        return Err(match first_failure {
            Some(error) => owner_failure(cancellation, error),
            None => contract_failure(cancellation),
        });
    }
    let mut first_failure = None;
    for (&permit, &outcome) in permits.iter().zip(outcomes) {
        if let Err(error) = owner
            .settle(permit, EffectSettlement::Tool { outcome })
            .await
            && first_failure.is_none()
        {
            first_failure = Some(error);
        }
    }
    match first_failure {
        Some(error) => Err(owner_failure(cancellation, error)),
        None => Ok(()),
    }
}

async fn settle_unstarted_permits(
    owner: &dyn EffectAdmission,
    permits: &[EffectPermit],
    cancellation: &CancellationToken,
) -> Result<(), AgentError> {
    let mut seen = HashSet::with_capacity(permits.len());
    let mut first_failure: Option<EffectOwnerFailure> = None;
    for &permit in permits {
        if !seen.insert(permit.get()) {
            continue;
        }
        if let Err(error) = owner
            .settle(
                permit,
                EffectSettlement::Tool {
                    outcome: EffectOutcome::NotStarted,
                },
            )
            .await
            && first_failure.is_none()
        {
            first_failure = Some(error);
        }
    }
    if let Some(error) = first_failure {
        return Err(owner_failure(cancellation, error));
    }
    Ok(())
}
