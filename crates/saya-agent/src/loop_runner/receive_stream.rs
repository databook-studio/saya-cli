mod consume;

use super::{attempt, attempt_metrics::AttemptTiming, emit};
use crate::{
    AgentEvent, AgentEventSink, AgentMetric, AgentMetricCount, AgentMetricOutcome,
    CancellationToken, ChatProvider, ChatRequest, ControlledProviderError, PhysicalAttemptControl,
    ProviderError, TokenUsage,
};

pub(super) async fn stream_attempt(
    provider: &dyn ChatProvider,
    request: &ChatRequest,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    retry: bool,
    physical_attempts: Option<&mut PhysicalAttemptControl<'_>>,
) -> attempt::AttemptOutcome {
    let mut timing = AttemptTiming::new();
    let outcome = if retry {
        sink.metric(AgentMetric::Count(AgentMetricCount::RetryAttemptStarted))
            .await;
        if cancellation.is_cancelled() {
            failed(attempt::AttemptError::Cancelled, None)
        } else {
            stream_attempt_inner(
                provider,
                request,
                sink,
                cancellation,
                events,
                &mut timing,
                physical_attempts,
            )
            .await
        }
    } else {
        stream_attempt_inner(
            provider,
            request,
            sink,
            cancellation,
            events,
            &mut timing,
            physical_attempts,
        )
        .await
    };
    timing.publish(sink, &outcome).await;
    outcome
}

async fn stream_attempt_inner(
    provider: &dyn ChatProvider,
    request: &ChatRequest,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    timing: &mut AttemptTiming,
    mut physical_attempts: Option<&mut PhysicalAttemptControl<'_>>,
) -> attempt::AttemptOutcome {
    emit(events, sink, AgentEvent::turn_started()).await;
    if cancellation.is_cancelled() {
        return failed(attempt::AttemptError::Cancelled, None);
    }
    let provider_request = request.clone();
    let provider_cancellation = cancellation.clone();
    let establishment_start = timing.begin_establishment();
    let establishment = if let Some(control) = physical_attempts.as_deref_mut() {
        provider
            .stream_with_physical_attempts(provider_request, provider_cancellation, control)
            .await
            .map_err(controlled_failure)
    } else {
        tokio::select! {
            _ = cancellation.cancelled() => {
                timing.finish_establishment(establishment_start, AgentMetricOutcome::Cancelled);
                return failed(attempt::AttemptError::Cancelled, None);
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
            if let Some(control) = physical_attempts.as_deref_mut()
                && let Err(owner_error) = control
                    .settle_pending_response(crate::EffectOutcome::Unknown, None)
                    .await
            {
                return failed(controlled_failure(owner_error), None);
            }
            return failed(error, None);
        }
    };
    let outcome = consume::consume_stream(stream, sink, cancellation, events, timing).await;
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

fn failed(error: attempt::AttemptError, usage: Option<TokenUsage>) -> attempt::AttemptOutcome {
    attempt::AttemptOutcome::Failed { error, usage }
}
