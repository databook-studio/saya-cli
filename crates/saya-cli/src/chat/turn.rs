//! One chat turn: the job engine behind a session's [`RunPort`].
//!
//! A session owns the run's life; this module is its one edge to the engine
//! that runs it. [`JobPort`] prepares the run, runs one turn per message, asks
//! the turn in flight to stop and closes the run — the four things [`RunPort`]
//! states — over the job's own entries, so nothing above it knows an epoch, a
//! slot or a lease.
//!
//! [`ChatTurn`] is that turn: the job's `run_next_turn` with the front's sink
//! wrapped in today's [`MetricsSink`] — the metric names a surface already
//! reads stay exactly what they were — read back as a [`ChatOutcome`] from the
//! turn's own stop, with the operator's cancel outranking it.
//!
//! Cancelling is the turn's and never the run's: the session's Esc stops the
//! child token of the turn in flight, so the next message is a fresh turn of
//! the same epoch, on the lease the run already holds.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, NoopEventSink};

use super::ChatOutcome;
use super::port::{PortError, Prepared, RunPort};
use crate::commands::coding::{CodingError, NativeCodingJob, TurnInputs};
use crate::local_metrics::LocalMetrics;
use crate::metrics_sink::MetricsSink;

/// The job engine behind one session's [`RunPort`].
pub(crate) struct JobPort {
    job: Arc<NativeCodingJob>,
    /// What the run is opened with, stated by the surface that composed it:
    /// taken by the one open, so a second prepare meets the job's own `Busy`.
    inputs: Mutex<Option<TurnInputs>>,
    /// The run's own counters, when the session has a scope to record into.
    metrics: Option<Arc<LocalMetrics>>,
    /// Whether the operator asked the turn in flight to stop: the job reports a
    /// cancelled turn as a failure, so this is what makes the outcome
    /// [`ChatOutcome::Cancelled`] and nothing else.
    cancelled: AtomicBool,
    /// Whether the run's one close has been stated.
    closed: AtomicBool,
}

impl JobPort {
    /// A port over `job`, opening its run with `inputs`.
    pub(crate) fn new(job: Arc<NativeCodingJob>, inputs: TurnInputs) -> Self {
        Self {
            job,
            inputs: Mutex::new(Some(inputs)),
            metrics: None,
            cancelled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    /// A port over a run a composition has already opened: `prepare` reports
    /// the run ready and opens nothing, because a second open of one run meets
    /// the job's own `Busy`. This is the surface that composes and opens a run
    /// itself (a `RunComposition`) and hands the session only its turns.
    pub(crate) fn opened(job: Arc<NativeCodingJob>) -> Self {
        Self {
            job,
            inputs: Mutex::new(None),
            metrics: None,
            cancelled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    /// The same port, recording every metric its turns report into `metrics`:
    /// the run's own counters, so a surface reads the same names it always did.
    pub(crate) fn with_metrics(mut self, metrics: Arc<LocalMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    fn inputs(&self) -> MutexGuard<'_, Option<TurnInputs>> {
        self.inputs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl RunPort for JobPort {
    async fn prepare(&self) -> Result<Prepared, String> {
        // The run's profile and workspace are the composition's to state; the
        // job states that it opened, which is what the session needs here.
        // `None` is a run the composition already opened: opening it a second
        // time would meet the job's own `Busy`.
        let Some(inputs) = self.inputs().take() else {
            return Ok(Prepared::default());
        };
        self.job
            .run_prepare(&NoopEventSink, inputs)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Prepared::default())
    }

    async fn next_turn(
        &self,
        sink: Arc<dyn AgentEventSink>,
        prompt: String,
    ) -> Result<ChatOutcome, PortError> {
        // This turn's cancel is the operator's, stated during it: a message
        // after it starts the next one uncancelled.
        self.cancelled.store(false, Ordering::Release);
        let turn = ChatTurn::new(&self.job, self.metrics.clone(), &self.cancelled);
        turn.run(sink.as_ref(), &prompt).await
    }

    async fn cancel_turn(&self) {
        // The turn's own stop signal, and never the run's: the run keeps its
        // epoch, its lease and its turns, so the next message is one of them.
        self.cancelled.store(true, Ordering::Release);
        if let Some(turn) = self.job.turn_cancellation() {
            turn.cancel();
        }
    }

    async fn close(&self) {
        // The run's close, stated once and only by the session's own quit: a
        // cancelled turn, a stopped one and a further message all leave the run
        // open.
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.job.run_close().await;
    }
}

/// One turn of a run: the job's `run_next_turn`, read back as a chat turn.
pub(crate) struct ChatTurn<'a> {
    job: &'a NativeCodingJob,
    /// The run's counters, if the session has a scope to record them in.
    metrics: Option<Arc<LocalMetrics>>,
    /// The port's memory of the operator's cancel, read once the turn ended: a
    /// cancelled turn reports a failure of its own, and this says which.
    cancelled: &'a AtomicBool,
}

impl<'a> ChatTurn<'a> {
    pub(crate) fn new(
        job: &'a NativeCodingJob,
        metrics: Option<Arc<LocalMetrics>>,
        cancelled: &'a AtomicBool,
    ) -> Self {
        Self {
            job,
            metrics,
            cancelled,
        }
    }

    /// Runs the turn: `prompt` is the operator's own words, and `sink` is the
    /// front's stream, which the run's metrics are recorded beside.
    pub(crate) async fn run(
        &self,
        sink: &dyn AgentEventSink,
        prompt: &str,
    ) -> Result<ChatOutcome, PortError> {
        // The front's own sink, with the run's counters recorded beside it: the
        // metrics reach the surface under the names it already reads.
        let metrics = MetricsSink::new(sink, self.metrics.clone());
        match self.job.run_next_turn(&metrics, prompt).await {
            Ok(_) => Ok(self.read()),
            Err(CodingError::Busy) => Err(PortError::Busy),
            Err(error) => {
                // A turn that ended on its own stop — the operator's cancel, a
                // guard, a failure — says so in the stop it latched. A refusal
                // that latched none (a finished run, a managed checkout) is the
                // engine's own sentence, for the front to show.
                match self.read() {
                    ChatOutcome::Answered => Err(PortError::Failed(error.to_string())),
                    ended => Ok(ended),
                }
            }
        }
    }

    /// How this turn ended, as the chat side of it.
    fn read(&self) -> ChatOutcome {
        ChatOutcome::from_stop(self.job.turn_stop(), self.cancelled.load(Ordering::Acquire))
    }
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
