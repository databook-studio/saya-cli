//! Owner-side end for a coordinator whose calls keep failing, whatever its
//! next action says.
use super::{CodingError, NativeCodingJob};
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AnsweringObservation, CancellationReason,
    ContextCompaction, ToolDiagnostic, ToolDiagnosticStatus,
};
use saya_types::{CodingFailureClass, CodingRunFailure};
use std::sync::{Mutex, OnceLock, PoisonError};

/// Consecutive root tool calls that FAILED, were REFUSED or were DENIED, with
/// no schedule live, before a run ends whatever its next action is. A direct
/// root does its own work in the working checkout, so a long session of
/// successful calls is legitimate rather than a stall, however little it
/// changed the job; a successful call resets the count. It is not unbounded: a
/// root whose every call fails, or is refused before it can have any effect,
/// is going nowhere and is ended. Eight times the handful of read-only calls a
/// coordinator needs to write its final answer leaves room for a long run of
/// inspection between changes.
pub(super) const DIRECT_STALL_LIMIT: u32 = 96;

/// Counts the calls since the last successful one.
#[derive(Debug, Default)]
pub(super) struct Streak {
    failed: u32,
}

impl Streak {
    /// Records one settled call and returns the length of the current run of
    /// consecutive failed, refused or denied calls. A successful call clears
    /// the run, so only a run of failures can ever reach the bound.
    pub(super) fn observe(&mut self, failed: bool) -> u32 {
        if failed {
            self.failed = self.failed.saturating_add(1);
        } else {
            self.failed = 0;
        }
        self.failed
    }
}

/// Wraps the root's sink to watch each settled tool call, including calls the
/// loop refuses before they execute.
pub(super) struct ConvergenceSink<'a> {
    job: &'a NativeCodingJob,
    downstream: &'a dyn AgentEventSink,
    watch: Mutex<Streak>,
    /// Set when the owner ended the root; a later end never replaces it.
    stopped: OnceLock<()>,
}

impl<'a> ConvergenceSink<'a> {
    pub(super) fn new(job: &'a NativeCodingJob, downstream: &'a dyn AgentEventSink) -> Self {
        Self {
            job,
            downstream,
            watch: Mutex::new(Streak::default()),
            stopped: OnceLock::new(),
        }
    }

    /// The failure of this run's end when the owner stopped the root.
    pub(super) fn failure(&self) -> Option<CodingRunFailure> {
        self.stopped
            .get()
            .map(|()| CodingRunFailure::root(CodingFailureClass::CoordinatorStalled))
    }

    /// Records one settled call's outcome. A call that failed, was refused or
    /// was denied lengthens the run of failures; a successful call clears it.
    /// The run ends once the run reaches the bound with no schedule live.
    async fn watch_call(&self, failed: bool) {
        // Without write paths there is no candidate to converge on: the
        // coordinator answers when it chooses to.
        if self.stopped.get().is_some()
            || self.job.request.write_paths.is_empty()
            || self.job.cancellation.is_cancelled()
        {
            return;
        }
        let failures = self
            .watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .observe(failed);
        if failures >= DIRECT_STALL_LIMIT && self.job.active.lock().await.is_none() {
            self.end();
        }
    }
}

impl ConvergenceSink<'_> {
    /// Records the reason first, so the root's cancellation is attributed to it.
    fn end(&self) {
        if self.stopped.set(()).is_ok() {
            self.job.stop_root(CodingRunFailure::root(
                CodingFailureClass::CoordinatorStalled,
            ));
        }
    }
}

#[async_trait]
impl AgentEventSink for ConvergenceSink<'_> {
    async fn observe_tool(&self, diagnostic: ToolDiagnostic) {
        // A requested call has no outcome yet; every settled call is either a
        // success, which clears the run of failures, or one of the three
        // failure outcomes, which lengthens it.
        let failed = match diagnostic.status() {
            ToolDiagnosticStatus::Requested => None,
            ToolDiagnosticStatus::Completed => Some(false),
            ToolDiagnosticStatus::Denied
            | ToolDiagnosticStatus::Failed
            | ToolDiagnosticStatus::Refused => Some(true),
        };
        self.downstream.observe_tool(diagnostic).await;
        if let Some(failed) = failed {
            self.watch_call(failed).await;
        }
    }

    fn observe_compaction(&self, observation: ContextCompaction) {
        self.downstream.observe_compaction(observation);
    }

    async fn emit(&self, event: AgentEvent) {
        self.downstream.emit(event).await;
    }

    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.downstream.observe_answering(observation).await;
    }

    async fn metric(&self, metric: AgentMetric) {
        self.downstream.metric(metric).await;
    }
}

impl NativeCodingJob {
    /// Records the stop as the root's cause, then cancels the root. The job's
    /// primary cause stays the highest-ranked class it latched — a stop latched
    /// later can therefore displace an earlier, lower-ranked cause. Nothing is
    /// applied, discarded or marked uncertain: the candidate, receipts and
    /// budget records are untouched.
    fn stop_root(&self, failure: CodingRunFailure) {
        self.latch_error(&CodingError::Failed(Box::new(failure)));
        self.cancellation
            .cancel_with_reason(CancellationReason::OwnerFailure);
    }
}

#[cfg(all(test, unix))]
#[path = "convergence_tests.rs"]
mod tests;
