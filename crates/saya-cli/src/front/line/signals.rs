//! The line front's one stop path: Ctrl-C stops the turn, never the run.
//!
//! A run outlives the turns it runs, so the operator's stop is the turn in
//! flight's own: the next line is a fresh turn of the same run, on the epoch and
//! the lease it already holds. What the stop also owes the surface is the one
//! `cancellation_to_local_settlement` sample — from the instant the stop was
//! accepted to the turn's own settlement — recorded here, once, before the
//! per-turn metrics section prints.

use std::sync::Arc;

use crate::agent::question_scope::QuestionScope;
use crate::local_metrics::LocalMetrics;
use crate::local_metrics_format::print_metrics_section;

/// The stop owner of one line session: the answering scope's accepted stop and
/// the run's counters.
pub(crate) struct Signals {
    /// The session's answering scope, when the run records one: its collector
    /// is the section a `--verbose` turn prints, and the stop instant it keeps
    /// is what the settlement sample is measured from.
    scope: Option<QuestionScope>,
}

impl Signals {
    pub(crate) fn new(scope: Option<QuestionScope>) -> Self {
        Self { scope }
    }

    /// Accepts one stop: the first wins and the answering source is cancelled,
    /// so a second Ctrl-C cannot move the instant the sample is measured from.
    /// The run's own token is never touched here — the caller stops the turn's.
    pub(crate) fn request_stop(&self) {
        if let Some(scope) = &self.scope {
            scope.request_stop();
        }
    }

    /// Says the turn's metrics section at its settlement: the sample a stop was
    /// waiting for is recorded here, and only a stop records one.
    pub(crate) fn settle(&self) {
        if let Some(scope) = &self.scope {
            scope.record_stop_settlement();
        }
        print_metrics_section(&self.metrics());
    }

    fn metrics(&self) -> Option<Arc<LocalMetrics>> {
        self.scope.as_ref().and_then(QuestionScope::metrics)
    }
}
