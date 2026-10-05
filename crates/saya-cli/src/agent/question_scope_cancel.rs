//! The application-side stop instant and its one settlement sample.
//!
//! `request_stop` is the one stop-request owner: the first stop wins and the
//! cancel propagates to the answering source. `record_stop_settlement` turns
//! a captured instant into the single `CancellationToLocalSettlement` sample
//! at the local settlement boundary; a core-originated stop (a budget
//! deadline latched inside the scope) has no request instant and records
//! nothing.

use saya_agent::{AgentMetric, AgentMetricOutcome, AgentMetricPhase};
use std::time::Instant;

use super::QuestionScope;

impl QuestionScope {
    /// Captures the stop instant once — the first stop wins, a second call
    /// never resets it — and cancels the answering source.
    pub(crate) fn request_stop(&self) {
        self.inner
            .stop_requested_at
            .lock()
            .expect("stop instant lock")
            .get_or_insert_with(Instant::now);
        self.cancellation().cancel();
    }

    /// Records the one cancellation-to-local-settlement sample: the elapsed
    /// time from the accepted stop instant to this local settlement call.
    /// No captured instant records nothing, and the sample is recorded
    /// exactly once.
    pub(crate) fn record_stop_settlement(&self) {
        let Some(metrics) = self.inner.metrics.clone() else {
            return;
        };
        let captured = self
            .inner
            .stop_requested_at
            .lock()
            .expect("stop instant lock")
            .take();
        let Some(captured) = captured else {
            return;
        };
        metrics.record(&AgentMetric::Phase {
            phase: AgentMetricPhase::CancellationToLocalSettlement,
            elapsed_ns: u64::try_from(captured.elapsed().as_nanos()).unwrap_or(u64::MAX),
            outcome: AgentMetricOutcome::Succeeded,
        });
    }
}

#[cfg(test)]
#[path = "question_scope_cancel_tests.rs"]
mod tests;
