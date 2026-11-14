use super::super::{add_usage, admission::Admission, attempt, attempt_metrics::duration_ns, emit};
use crate::{
    AgentEvent, AgentEventSink, AgentMetric, AgentMetricCount, AgentMetricOutcome,
    AgentMetricPhase, CancellationToken, ProviderRecoveryPhase, ProviderRecoveryReason, TokenUsage,
};
use std::time::Duration;

/// The part of a job's wall clock a retry may not spend. A retry is only worth
/// starting when more than this is left: the next attempt needs room to
/// establish and stream, and a job that would be cut mid-answer has already
/// lost the work the reserve protects.
pub(super) const RETRY_RESERVE: Duration = Duration::from_secs(30);

/// What a retry may spend: the job's remaining wall clock less the reserve.
/// `None` when the job has no wall clock.
fn retry_budget(remaining: Option<Duration>) -> Option<Duration> {
    remaining.map(|remaining| remaining.saturating_sub(RETRY_RESERVE))
}

/// A retry needs more than the reserve in hand. At exactly the reserve there
/// is no time for the attempt, so the job stops instead.
fn retry_allowed(remaining: Option<Duration>) -> bool {
    retry_budget(remaining).is_none_or(|budget| !budget.is_zero())
}

/// The backoff never sleeps into the reserve; the attempt it precedes needs
/// what is left.
fn clamped_backoff(delay: Duration, remaining: Option<Duration>) -> Duration {
    retry_budget(remaining).map_or(delay, |budget| delay.min(budget))
}

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

/// The receive's shared state a retry decision reads and writes. Grouped so
/// the plan signature stays readable.
pub(super) struct RetryContext<'a> {
    pub(super) admission: &'a mut Admission,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: &'a CancellationToken,
    pub(super) events: &'a mut Vec<AgentEvent>,
}

/// Classifies a failed attempt into the provider-recovery events and either a
/// further retry after the scheduled backoff delay or the error that ends the
/// receive. Emission order and the cancellation checks are the loop's
/// contract: the reset, the recovery phase, the backoff metric, and only then
/// the next attempt.
pub(super) async fn plan_retry(
    error: attempt::AttemptError,
    retries: u8,
    delays: &[Duration],
    wall_deadline: Option<tokio::time::Instant>,
    context: RetryContext<'_>,
) -> RetryOutcome {
    let RetryContext {
        admission,
        sink,
        cancellation,
        events,
    } = context;
    let limit = super::delays::max_attempts(delays);
    match error {
        attempt::AttemptError::Retryable { error: _, reason }
            if usize::from(retries) < delays.len() =>
        {
            let remaining = remaining_wall(wall_deadline);
            if !retry_allowed(remaining) {
                return stop_on_the_clock(events, sink, retries, limit, cancellation).await;
            }
            let next_attempt = retries.saturating_add(2);
            emit(events, sink, AgentEvent::turn_reset()).await;
            emit(
                events,
                sink,
                AgentEvent::provider_recovery(
                    ProviderRecoveryPhase::Retrying,
                    reason,
                    next_attempt,
                    limit,
                ),
            )
            .await;
            let backoff_started = std::time::Instant::now();
            let delay = clamped_backoff(delays[usize::from(retries)], remaining);
            let waited = wait(delay, cancellation).await;
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
                        limit,
                    ),
                )
                .await;
                return RetryOutcome::GiveUp(error);
            }
            // The clamped sleep may have spent the last of the budget.
            if !retry_allowed(remaining_wall(wall_deadline)) {
                return stop_on_the_clock(events, sink, retries, limit, cancellation).await;
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
                    limit,
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
                    limit,
                ),
            )
            .await;
            RetryOutcome::GiveUp(error)
        }
    }
}

/// What the job has left of its wall clock, read at the decision point.
fn remaining_wall(deadline: Option<tokio::time::Instant>) -> Option<Duration> {
    deadline.map(|deadline| deadline.saturating_duration_since(tokio::time::Instant::now()))
}

/// Ends the receive under the job's elapsed-time budget: the same latched
/// reason a deadline expiry uses, so the run reports the budget stop rather
/// than a provider failure.
async fn stop_on_the_clock(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    retries: u8,
    limit: u8,
    cancellation: &CancellationToken,
) -> RetryOutcome {
    cancellation.cancel_with_reason(crate::CancellationReason::Budget(
        crate::BudgetDimension::ElapsedTime,
    ));
    emit(
        events,
        sink,
        AgentEvent::provider_recovery(
            ProviderRecoveryPhase::NotRetried,
            ProviderRecoveryReason::Cancelled,
            retries.saturating_add(1),
            limit,
        ),
    )
    .await;
    RetryOutcome::GiveUp(crate::AgentError::Cancelled)
}

/// Sleeps `delay`, aborting locally when the run is cancelled.
async fn wait(delay: Duration, cancellation: &CancellationToken) -> Result<(), crate::AgentError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(crate::AgentError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{RETRY_RESERVE, clamped_backoff, retry_allowed, retry_budget};
    use std::time::Duration;

    /// A retry needs more than the reserve in hand. Exactly the reserve is
    /// already too late: the attempt would be cut before it answered.
    #[test]
    fn a_retry_inside_the_reserve_is_not_allowed() {
        assert!(!retry_allowed(Some(RETRY_RESERVE - Duration::from_secs(1))));
        assert!(!retry_allowed(Some(RETRY_RESERVE)));
        assert!(retry_allowed(Some(RETRY_RESERVE + Duration::from_secs(1))));
    }

    /// No wall clock means no reserve: the retry schedule is unchanged.
    #[test]
    fn an_uncapped_job_has_no_retry_reserve() {
        assert!(retry_allowed(None));
        assert_eq!(retry_budget(None), None);
        assert_eq!(
            clamped_backoff(Duration::from_secs(8), None),
            Duration::from_secs(8)
        );
    }

    /// The backoff sleeps only what is left above the reserve.
    #[test]
    fn the_backoff_is_clamped_to_the_budget() {
        let remaining = Some(RETRY_RESERVE + Duration::from_secs(3));
        assert_eq!(
            clamped_backoff(Duration::from_secs(8), remaining),
            Duration::from_secs(3)
        );
        assert_eq!(
            clamped_backoff(Duration::from_secs(1), remaining),
            Duration::from_secs(1)
        );
    }
}
