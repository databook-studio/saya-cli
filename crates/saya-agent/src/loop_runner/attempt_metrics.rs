use super::attempt::{AttemptError, AttemptOutcome};
use crate::{
    AgentAttemptOutcome, AgentEventSink, AgentMetric, AgentMetricOutcome, AgentMetricPhase,
};
use std::time::{Duration, Instant};

pub(super) struct AttemptTiming {
    started: Instant,
    establishment_started: Option<Instant>,
    establishment: Option<(u64, AgentMetricOutcome)>,
    stream_established: bool,
    read_elapsed_ns: u64,
    first_event: Option<(u64, AgentMetricOutcome)>,
}

impl AttemptTiming {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            establishment_started: None,
            establishment: None,
            stream_established: false,
            read_elapsed_ns: 0,
            first_event: None,
        }
    }

    pub(super) fn begin_establishment(&mut self) -> Instant {
        let start = Instant::now();
        self.establishment_started = Some(start);
        start
    }

    pub(super) fn finish_establishment(&mut self, start: Instant, outcome: AgentMetricOutcome) {
        self.establishment = Some((duration_ns(start.elapsed()), outcome));
        self.stream_established = outcome == AgentMetricOutcome::Succeeded;
    }

    pub(super) fn add_read_elapsed(&mut self, elapsed: Duration) {
        self.read_elapsed_ns = self.read_elapsed_ns.saturating_add(duration_ns(elapsed));
    }

    pub(super) fn first_event(&mut self, outcome: AgentMetricOutcome) {
        if self.first_event.is_none()
            && let Some(start) = self.establishment_started
        {
            self.first_event = Some((duration_ns(start.elapsed()), outcome));
        }
    }

    pub(super) async fn publish(self, sink: &dyn AgentEventSink, outcome: &AttemptOutcome) {
        let elapsed_ns = duration_ns(self.started.elapsed());
        let (attempt_outcome, phase_outcome, usage) = match outcome {
            AttemptOutcome::Success { usage, .. } => (
                AgentAttemptOutcome::Succeeded,
                AgentMetricOutcome::Succeeded,
                *usage,
            ),
            AttemptOutcome::Failed { error, usage } => {
                let cancelled = matches!(error, AttemptError::Cancelled);
                (
                    if cancelled {
                        AgentAttemptOutcome::Cancelled
                    } else {
                        AgentAttemptOutcome::Failed
                    },
                    if cancelled {
                        AgentMetricOutcome::Cancelled
                    } else {
                        AgentMetricOutcome::Failed
                    },
                    *usage,
                )
            }
        };
        let (establishment_ns, establishment_outcome) = self
            .establishment
            .unwrap_or((0, AgentMetricOutcome::Unavailable));
        let (first_event_ns, first_event_outcome) = self
            .first_event
            .unwrap_or((0, AgentMetricOutcome::Unavailable));
        let read_outcome = if self.stream_established {
            phase_outcome
        } else {
            AgentMetricOutcome::Unavailable
        };
        for metric in [
            AgentMetric::Phase {
                phase: AgentMetricPhase::ProviderEstablishment,
                elapsed_ns: establishment_ns,
                outcome: establishment_outcome,
            },
            AgentMetric::Phase {
                phase: AgentMetricPhase::ProviderReadAwait,
                elapsed_ns: self.read_elapsed_ns,
                outcome: read_outcome,
            },
            AgentMetric::Phase {
                phase: AgentMetricPhase::FirstProviderEvent,
                elapsed_ns: first_event_ns,
                outcome: first_event_outcome,
            },
            AgentMetric::Phase {
                phase: AgentMetricPhase::LocalAttempt,
                elapsed_ns,
                outcome: phase_outcome,
            },
            AgentMetric::UsageCoverage {
                attempt_outcome,
                reported: usage.is_some(),
            },
        ] {
            sink.metric(metric).await;
        }
    }
}

pub(super) fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
#[path = "attempt_metrics_tests.rs"]
mod tests;
