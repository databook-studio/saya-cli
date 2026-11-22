//! The line front's one stop path: Ctrl-C stops the turn, never the run.
//!
//! A run outlives the turns it runs, so the operator's stop is the turn in
//! flight's own: the next line is a fresh turn of the same run, on the epoch and
//! the lease it already holds. What the stop also owes the surface is the one
//! `cancellation_to_local_settlement` sample — from the instant the stop was
//! accepted to the turn's own settlement — recorded here, once, before the
//! per-turn metrics section prints.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::CancellationToken;

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

    /// Starts the next question's collector: a question's counters are its own,
    /// so the section a settled turn prints holds this question's samples and
    /// not the session's. A clarification reply is the same question and never
    /// calls this; the turn after an answered question does.
    pub(crate) fn begin_question(&self) {
        if let Some(metrics) = self.metrics() {
            metrics.begin_question();
        }
    }

    /// The run's counters, when the session records a scope: the collector a
    /// settled turn's section prints, and the one the question's wait samples
    /// go to.
    pub(crate) fn metrics(&self) -> Option<Arc<LocalMetrics>> {
        self.scope.as_ref().and_then(QuestionScope::metrics)
    }

    /// The answering source's own stop, when the run records a scope: the token
    /// a question's elapsed budget cancels from inside the scope.
    pub(crate) fn cancellation(&self) -> Option<CancellationToken> {
        self.scope.as_ref().map(QuestionScope::cancellation)
    }

    /// The elapsed time the question has left, when it has a bound: the longest
    /// one wait for an answer may take, since the scope debits only between the
    /// source segments and cannot end a prompt nobody answers.
    pub(crate) fn remaining_elapsed(&self) -> Option<Duration> {
        self.scope
            .as_ref()
            .and_then(QuestionScope::remaining_elapsed)
    }
}
