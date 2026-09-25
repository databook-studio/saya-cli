//! The TUI run panel's state: what a run driven from the session keeps.
//!
//! A run started here is a worker task (`run_worker.rs`), so the event loop
//! stays responsive while it runs. The state the panel holds: the plan's
//! steps with their live status and elapsed times, the last lifecycle line
//! (shaped by the shared `run_event_text` renderer — the panel formats
//! nothing the wire and `saya run log` do not already agree on), and the
//! episode's own transcript, fed by the same `stream_events::apply_event`
//! the session's conversation uses — so an episode never interleaves with
//! the session's own turns, and both render alike. The drain and the `App`
//! seams live in `run_panel_apply.rs` and `application/run_panel.rs`.

use super::run_worker::RunWorker;
use super::transcript::Transcript;
use std::time::{Duration, Instant};

/// Where one plan step stands, as the panel last saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// One row of the step list: the goal the approval view carried, its live
/// status, and the elapsed time the panel measured (frozen at the step's
/// boundary event, ticking while it runs).
#[derive(Debug)]
pub(crate) struct RunStep {
    pub(crate) goal: String,
    pub(crate) status: RunStepStatus,
    pub(crate) elapsed: Option<Duration>,
    /// Set when the step's `StepStarted` arrived; taken when the boundary
    /// event freezes the elapsed time.
    pub(crate) started: Option<Instant>,
}

/// A plan-approval ask waiting on the panel's modal. `respond` answers the
/// drive's one decision — only an explicit yes approves, exactly the rule
/// the tool-approval modal applies.
pub(crate) struct PendingPlanApproval {
    pub(crate) view_text: String,
    pub(crate) respond: tokio::sync::oneshot::Sender<bool>,
}

/// The run panel: the plan's steps with their live status and elapsed
/// times, the last lifecycle line, and the episode's own transcript.
pub(crate) struct RunPanel {
    pub(crate) run_id: String,
    pub(crate) goal: String,
    pub(crate) worker: Option<RunWorker>,
    pub(crate) steps: Vec<RunStep>,
    /// The last lifecycle event's line, shaped by the shared `run_event_text`
    /// — the same bytes the wire and `saya run log` render.
    pub(crate) status: String,
    /// Whether the status names a failure the user should read as one.
    pub(crate) status_is_error: bool,
    /// The run paused — resumable, so its status neither reads as success
    /// nor as failure.
    pub(crate) paused: bool,
    /// A terminal event arrived (paused, completed, failed, cancelled); a
    /// later `Done` message is then redundant, not news.
    pub(crate) terminated: bool,
    /// The panel asked the worker to stop; the status row says so until the
    /// `Cancelled` event lands.
    pub(crate) cancelling: bool,
    /// The panel's spinner frame, advanced by each drain so the live status
    /// animates while a run is in flight.
    pub(crate) spinner: usize,
    /// The episode's own transcript — never the session's conversation.
    pub(crate) episode: Transcript,
    pub(crate) started: Option<Instant>,
    pub(crate) plan_approval: Option<PendingPlanApproval>,
}

impl RunPanel {
    pub(crate) fn new(worker: RunWorker, run_id: String, goal: String) -> Self {
        Self {
            run_id,
            goal,
            worker: Some(worker),
            steps: Vec::new(),
            status: String::new(),
            status_is_error: false,
            paused: false,
            terminated: false,
            cancelling: false,
            spinner: 0,
            episode: Transcript::new(),
            started: None,
            plan_approval: None,
        }
    }

    /// The run is in flight: a worker is still driving it.
    pub(crate) fn is_active(&self) -> bool {
        self.worker.is_some()
    }
}

/// The channel pair a test wires a panel with, exactly the shape the real
/// worker hands back. Tests drive events through the same drain.
#[cfg(test)]
pub(crate) fn test_channels() -> (
    std::sync::mpsc::Sender<super::run_worker::RunMsg>,
    std::sync::mpsc::Receiver<super::run_worker::RunMsg>,
) {
    std::sync::mpsc::channel()
}
