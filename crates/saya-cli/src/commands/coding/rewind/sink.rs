//! The turn boundary a checkpoint is taken at.
//!
//! The root loop emits [`AgentEvent::TurnStarted`] once before each provider
//! attempt, so the sink wrapper takes the job's checkpoint there: at that
//! moment the previous turn's tool calls have landed and this turn's have not,
//! which is exactly "the working checkout after the last turn that changed
//! files". A retried or continued attempt re-emits the event with the checkout
//! untouched, and the capture skips a state the newest checkpoint already
//! holds.
//!
//! Capture never fails a run: it is the job's own
//! [`NativeCodingJob::capture_rewind_at`], which records a turn it could not
//! checkpoint in the rewind store the operator lists, and returns.

use crate::commands::coding::NativeCodingJob;
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AnsweringObservation, ContextCompaction,
    ToolDiagnostic,
};
use saya_harness::coding::worktree::VerifiedCheckout;
use std::sync::Arc;

/// Wraps the root's sink to take one checkpoint per turn boundary.
pub(crate) struct RewindSink<'a> {
    job: &'a NativeCodingJob,
    downstream: &'a dyn AgentEventSink,
    /// The run's own working checkout, resolved once before the loop. The root
    /// tools write this one directory for the whole run, so re-proving its
    /// ownership at every turn boundary would spend a git round-trip per turn
    /// on a path that cannot change identity mid-run. `None` is a job with no
    /// working checkout at all.
    checkout: Option<Arc<VerifiedCheckout>>,
}

impl<'a> RewindSink<'a> {
    pub(crate) fn new(
        job: &'a NativeCodingJob,
        downstream: &'a dyn AgentEventSink,
        checkout: Option<Arc<VerifiedCheckout>>,
    ) -> Self {
        Self {
            job,
            downstream,
            checkout,
        }
    }

    /// The working checkout this run resolved before the loop. The run end
    /// takes its final checkpoint from the same resolution instead of
    /// inventorying W again.
    pub(crate) fn checkout(&self) -> Option<&VerifiedCheckout> {
        self.checkout.as_deref()
    }
}

#[async_trait]
impl AgentEventSink for RewindSink<'_> {
    fn observe_compaction(&self, observation: ContextCompaction) {
        self.downstream.observe_compaction(observation);
    }

    async fn observe_tool(&self, diagnostic: ToolDiagnostic) {
        self.downstream.observe_tool(diagnostic).await;
    }

    async fn emit(&self, event: AgentEvent) {
        let boundary = matches!(event, AgentEvent::TurnStarted);
        // The event is forwarded first: the checkpoint is a copy of the
        // checkout, bounded but not free, and nothing downstream should wait on
        // it. Both orders sit at the same boundary — before this turn's calls.
        self.downstream.emit(event).await;
        if boundary {
            let started = std::time::Instant::now();
            self.job.capture_rewind_at(self.checkout.as_deref()).await;
            self.job
                .lifecycle()
                .note_elapsed("rewind_checkpoint", started.elapsed());
        }
    }

    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.downstream.observe_answering(observation).await;
    }

    async fn metric(&self, metric: AgentMetric) {
        self.downstream.metric(metric).await;
    }
}
