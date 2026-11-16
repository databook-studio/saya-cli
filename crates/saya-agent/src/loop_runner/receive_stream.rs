mod consume;

use super::{attempt, attempt_metrics::AttemptTiming, emit};
use crate::{
    AgentEvent, AgentEventSink, AgentMetric, AgentMetricCount, AgentMetricOutcome,
    CancellationToken, ChatProvider, ChatRequest, ControlledProviderError, PhysicalAttemptControl,
    ProviderError, ProviderStream, TokenUsage,
};
use std::time::Duration;

/// One attempt's total deadline: the smaller of the provider's cap and what
/// remains of the job's wall clock. For a retry that remainder excludes the
/// retry reserve. It is recomputed for each attempt, so a retry spends the
/// budget that is actually left instead of the one the logical request opened
/// with.
#[derive(Clone, Copy)]
pub(super) struct RequestDeadline {
    pub(super) at: tokio::time::Instant,
    pub(super) limit: Duration,
    /// The job's wall clock, not the provider's cap, set the deadline. Its
    /// expiry is the job's budget stopping, never a provider failure.
    pub(super) wall_bound: bool,
}

impl RequestDeadline {
    pub(super) fn new(cap: Duration, remaining_wall: Option<Duration>) -> Self {
        let limit = remaining_wall.map_or(cap, |wall| wall.min(cap));
        Self {
            at: tokio::time::Instant::now() + limit,
            limit,
            wall_bound: remaining_wall.is_some_and(|wall| wall <= cap),
        }
    }

    pub(super) fn expired(&self) -> bool {
        tokio::time::Instant::now() >= self.at
    }
}

/// Everything one physical attempt sends and is bounded by. Grouped so the
/// attempt functions keep a readable argument list.
pub(super) struct StreamAttempt<'a> {
    pub(super) provider: &'a dyn ChatProvider,
    pub(super) request: &'a ChatRequest,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: &'a CancellationToken,
    pub(super) retry: bool,
    pub(super) deadline: RequestDeadline,
}

pub(super) async fn stream_attempt(
    attempt: StreamAttempt<'_>,
    events: &mut Vec<AgentEvent>,
    physical_attempts: Option<&mut PhysicalAttemptControl<'_>>,
) -> attempt::AttemptOutcome {
    let mut timing = AttemptTiming::new();
    let sink = attempt.sink;
    let outcome = if attempt.retry {
        sink.metric(AgentMetric::Count(AgentMetricCount::RetryAttemptStarted))
            .await;
        if attempt.cancellation.is_cancelled() {
            failed(attempt::AttemptError::Cancelled, None)
        } else {
            stream_attempt_inner(attempt, events, &mut timing, physical_attempts).await
        }
    } else {
        stream_attempt_inner(attempt, events, &mut timing, physical_attempts).await
    };
    timing.publish(sink, &outcome).await;
    outcome
}

async fn stream_attempt_inner(
    attempt: StreamAttempt<'_>,
    events: &mut Vec<AgentEvent>,
    timing: &mut AttemptTiming,
    mut physical_attempts: Option<&mut PhysicalAttemptControl<'_>>,
) -> attempt::AttemptOutcome {
    let StreamAttempt {
        provider,
        request,
        sink,
        cancellation,
        retry: _,
        deadline,
    } = attempt;
    emit(events, sink, AgentEvent::turn_started()).await;
    if cancellation.is_cancelled() {
        return failed(attempt::AttemptError::Cancelled, None);
    }
    let provider_request = request.clone();
    let provider_cancellation = cancellation.clone();
    let establishment_start = timing.begin_establishment();
    let establishment = if let Some(control) = physical_attempts.as_deref_mut() {
        // The controlled path gets the same bound. The provider future is
        // dropped when the deadline fires, so whatever it admitted is settled
        // here rather than left dangling.
        let ended = tokio::select! {
            _ = cancellation.cancelled() => Establishment::Cancelled,
            result = tokio::time::timeout_at(
                deadline.at,
                provider.stream_with_physical_attempts(
                    provider_request,
                    provider_cancellation,
                    &mut *control,
                ),
            ) => match result {
                Ok(result) => Establishment::Returned(result.map_err(controlled_failure)),
                Err(_) => Establishment::Deadline,
            },
        };
        match ended {
            Establishment::Returned(result) => result,
            Establishment::Cancelled => {
                timing.finish_establishment(establishment_start, AgentMetricOutcome::Cancelled);
                if let Err(owner_error) = control.settle_aborted_establishment().await {
                    return failed(controlled_failure(owner_error), None);
                }
                return failed(attempt::AttemptError::Cancelled, None);
            }
            Establishment::Deadline => {
                timing.finish_establishment(establishment_start, AgentMetricOutcome::Failed);
                if let Err(owner_error) = control.settle_aborted_establishment().await {
                    return failed(controlled_failure(owner_error), None);
                }
                return failed(
                    consume::deadline_failure(deadline, cancellation, deadline.limit, 0),
                    None,
                );
            }
        }
    } else {
        tokio::select! {
            _ = cancellation.cancelled() => {
                timing.finish_establishment(establishment_start, AgentMetricOutcome::Cancelled);
                return failed(attempt::AttemptError::Cancelled, None);
            }
            _ = tokio::time::sleep_until(deadline.at) => {
                timing.finish_establishment(establishment_start, AgentMetricOutcome::Failed);
                return failed(
                    consume::deadline_failure(deadline, cancellation, deadline.limit, 0),
                    None,
                );
            }
            result = provider.stream(provider_request, provider_cancellation) => {
                result.map_err(establishment_failure)
            }
        }
    };
    let stream = match establishment {
        Ok(stream) => {
            timing.finish_establishment(establishment_start, AgentMetricOutcome::Succeeded);
            stream
        }
        Err(error) => {
            let phase_outcome = if matches!(&error, attempt::AttemptError::Cancelled) {
                AgentMetricOutcome::Cancelled
            } else {
                AgentMetricOutcome::Failed
            };
            timing.finish_establishment(establishment_start, phase_outcome);
            // A failed establishment may leave a dispatched send unsettled
            // (including one whose provider future this select dropped on
            // cancellation). The loop owns that settlement here, outside any
            // select, so it cannot be cancelled midway.
            if let Some(control) = physical_attempts.as_deref_mut()
                && let Err(owner_error) = control.settle_aborted_establishment().await
            {
                return failed(controlled_failure(owner_error), None);
            }
            return failed(error, None);
        }
    };
    // Establishment may have won a race with an already-expired deadline.
    // Consuming then would spend a continuation on a request whose clock is
    // gone; answer the deadline once, here, instead.
    if deadline.expired() {
        if let Some(control) = physical_attempts.as_deref_mut()
            && let Err(owner_error) = control
                .settle_pending_response(crate::EffectOutcome::Unknown, None)
                .await
        {
            return failed(controlled_failure(owner_error), None);
        }
        return failed(
            consume::deadline_failure(deadline, cancellation, deadline.limit, 0),
            None,
        );
    }
    let outcome =
        consume::consume_stream(stream, sink, cancellation, events, timing, deadline).await;
    if let Some(control) = physical_attempts {
        let (outcome, usage) = match &outcome {
            attempt::AttemptOutcome::Success { usage, .. } => {
                (crate::EffectOutcome::Succeeded, *usage)
            }
            // A complete output-limit truncation is a known provider answer
            // (the stream ended with its terminal truncation signal). Every
            // other failure, a transport error, an idle cut or a malformed
            // stream, leaves the client unable to know the outcome.
            attempt::AttemptOutcome::Failed { error, usage } => (
                if is_known_truncation(error) {
                    crate::EffectOutcome::Succeeded
                } else {
                    crate::EffectOutcome::Unknown
                },
                *usage,
            ),
        };
        if let Err(error) = control.settle_pending_response(outcome, usage).await {
            return failed(controlled_failure(error), usage);
        }
    }
    outcome
}

pub(super) fn is_known_truncation(error: &attempt::AttemptError) -> bool {
    matches!(
        error,
        attempt::AttemptError::Terminal {
            error: ProviderError::OutputTruncated { .. },
            ..
        }
    )
}

fn controlled_failure(error: ControlledProviderError) -> attempt::AttemptError {
    match error {
        ControlledProviderError::Provider(error) => establishment_failure(error),
        ControlledProviderError::UnsupportedCoverage | ControlledProviderError::AttemptLimit => {
            attempt::AttemptError::UnsupportedPhysicalCoverage
        }
        ControlledProviderError::AdmissionDenied => attempt::AttemptError::AdmissionDenied,
        ControlledProviderError::OwnerFailure(error) => attempt::AttemptError::OwnerFailure(error),
    }
}

fn establishment_failure(error: ProviderError) -> attempt::AttemptError {
    match consume::provider_failure(error) {
        attempt::AttemptError::Retryable { error, reason } => consume::terminal(error, reason),
        error => error,
    }
}

/// How establishment ended before a stream existed.
enum Establishment {
    Returned(Result<ProviderStream, attempt::AttemptError>),
    Cancelled,
    Deadline,
}

fn failed(error: attempt::AttemptError, usage: Option<TokenUsage>) -> attempt::AttemptOutcome {
    attempt::AttemptOutcome::Failed { error, usage }
}
