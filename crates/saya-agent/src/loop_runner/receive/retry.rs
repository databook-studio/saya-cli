use super::super::{add_usage, admission::Admission, attempt, attempt_metrics::duration_ns, emit};
use crate::{
    AgentEvent, AgentEventSink, AgentMetric, AgentMetricCount, AgentMetricOutcome,
    AgentMetricPhase, CancellationToken, ProviderRecoveryPhase, ProviderRecoveryReason, TokenUsage,
};
use std::time::Duration;

/// The decision for a failed provider attempt: retry after backoff with the
/// returned attempt count, or give up with the settled error.
pub(super) enum RetryOutcome {
    /// Backoff elapsed; the receive loop retries with the returned count.
    Retry(u8),
    /// The receive failed with the reported error.
    GiveUp(crate::AgentError),
}

/// Charges a failed attempt's reported usage and settles it against the
/// admission budget. A truncated output is announced and, when the settlement
/// or a fresh admission check fails, ends the receive; otherwise the caller
/// proceeds to the retry decision. Returns the failure that ends the receive,
/// or `None` when retry planning should run.
pub(super) async fn settle_failed_usage(
    error: &attempt::AttemptError,
    usage: Option<TokenUsage>,
    admission: &mut Admission,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    recovered_usage: &mut TokenUsage,
) -> Option<attempt::ReceiveFailure> {
    let output_truncated = matches!(
        error,
        attempt::AttemptError::Terminal {
            error: crate::ProviderError::OutputTruncated { .. },
            ..
        }
    );
    let mut settlement_error = None;
    if let Some(usage) = usage {
        emit(events, sink, AgentEvent::failed_attempt_usage(usage)).await;
        add_usage(recovered_usage, usage);
        settlement_error = admission.settle(Some(usage), cancellation).err();
    }
    if output_truncated {
        sink.metric(AgentMetric::Count(AgentMetricCount::OutputTruncated))
            .await;
        if let Some(error) = settlement_error {
            return Some(attempt::ReceiveFailure {
                error,
                usage: *recovered_usage,
            });
        }
        if let Err(error) = admission.check(cancellation) {
            return Some(attempt::ReceiveFailure {
                error,
                usage: *recovered_usage,
            });
        }
    } else if let Some(error) = settlement_error {
        return Some(attempt::ReceiveFailure {
            error,
            usage: *recovered_usage,
        });
    }
    None
}

/// Classifies a failed attempt into the provider-recovery events and either a
/// further retry after the fixed backoff delay or the error that ends the
/// receive. Emission order and the cancellation checks are the loop's
/// contract: the reset, the recovery phase, the backoff metric, and only then
/// the next attempt.
pub(super) async fn plan_retry(
    error: attempt::AttemptError,
    retries: u8,
    delays: &[Duration],
    admission: &mut Admission,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
) -> RetryOutcome {
    match error {
        attempt::AttemptError::Retryable { error: _, reason } if retries < attempt::RETRY_LIMIT => {
            let next_attempt = retries.saturating_add(2);
            emit(events, sink, AgentEvent::turn_reset()).await;
            emit(
                events,
                sink,
                AgentEvent::provider_recovery(
                    ProviderRecoveryPhase::Retrying,
                    reason,
                    next_attempt,
                    attempt::MAX_ATTEMPTS,
                ),
            )
            .await;
            let backoff_started = std::time::Instant::now();
            let waited = wait(delays[usize::from(retries)], cancellation).await;
            sink.metric(AgentMetric::Phase {
                phase: AgentMetricPhase::RetryBackoff,
                elapsed_ns: duration_ns(backoff_started.elapsed()),
                outcome: if waited.is_ok() {
                    AgentMetricOutcome::Succeeded
                } else {
                    AgentMetricOutcome::Cancelled
                },
            })
            .await;
            if let Err(error) = waited {
                emit(
                    events,
                    sink,
                    AgentEvent::provider_recovery(
                        ProviderRecoveryPhase::NotRetried,
                        ProviderRecoveryReason::Cancelled,
                        retries.saturating_add(1),
                        attempt::MAX_ATTEMPTS,
                    ),
                )
                .await;
                return RetryOutcome::GiveUp(error);
            }
            if let Err(error) = admission.check(cancellation) {
                return RetryOutcome::GiveUp(error);
            }
            RetryOutcome::Retry(retries.saturating_add(1))
        }
        attempt::AttemptError::Retryable { error, reason } => {
            emit(
                events,
                sink,
                AgentEvent::provider_recovery(
                    ProviderRecoveryPhase::Exhausted,
                    reason,
                    retries.saturating_add(1),
                    attempt::MAX_ATTEMPTS,
                ),
            )
            .await;
            RetryOutcome::GiveUp(crate::AgentError::Provider(error))
        }
        error => {
            let reason = error.reason();
            let error = error.error();
            emit(
                events,
                sink,
                AgentEvent::provider_recovery(
                    ProviderRecoveryPhase::NotRetried,
                    reason,
                    retries.saturating_add(1),
                    attempt::MAX_ATTEMPTS,
                ),
            )
            .await;
            RetryOutcome::GiveUp(error)
        }
    }
}

/// Sleeps `delay`, aborting locally when the run is cancelled.
async fn wait(delay: Duration, cancellation: &CancellationToken) -> Result<(), crate::AgentError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(crate::AgentError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}
