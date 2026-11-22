mod consent;
mod pump;
mod worker;
use crate::cli::CodingCommand;
use saya_agent::{ApprovalChoice, CancellationToken};
use std::{sync::mpsc, thread::JoinHandle};
use tokio::sync::oneshot;

/// What a panel's control mailbox says when it is closed or full: cancellation
/// stays available either way.
pub(crate) const CONTROL_FULL: &str =
    "Job control mailbox is full or closed; cancellation remains available";
/// What a surface says when an approval is asked for a job that has none
/// pending: the panel's own words, so both surfaces refuse in one sentence.
pub(crate) const NO_PENDING: &str = "No pending approval for that job";
/// What a surface says when a job's worker went away without a result.
pub(crate) const WORKER_GONE: &str =
    "Job worker ended without a settled result; inspect recovery before continuing.";
/// What a panel's worker has said between two looks, as a surface reads it.
pub(crate) use pump::Said;
/// Opening a job is today's worker, re-exported.
pub(crate) use worker::spawn as spawn_job;

pub(crate) enum CodingMessage {
    Activity(String),
    Approval {
        text: String,
        respond: oneshot::Sender<bool>,
    },
    /// One egressing call a job started from this TUI puts to the operator:
    /// the caller's own subject line and the call's target, answered through
    /// the chat's approval prompt — never the panel's text question.
    Question {
        subject: String,
        target: String,
        respond: oneshot::Sender<ApprovalChoice>,
    },
}
pub(crate) struct CodingPanel {
    id: Option<String>,
    rx: mpsc::Receiver<CodingMessage>,
    /// Control receipts on their own bounded queue, drained after activity so a
    /// receipt is the newest block of its tick and the next frame paints it.
    receipts: mpsc::Receiver<CodingMessage>,
    done: mpsc::Receiver<Result<String, String>>,
    commands: tokio::sync::mpsc::Sender<CodingCommand>,
    cancellation: CancellationToken,
    pending: Option<oneshot::Sender<bool>>,
    worker: Option<JoinHandle<()>>,
    active: bool,
}
impl CodingPanel {
    /// Cancels the job this id names: its own token when it is the one, the
    /// control mailbox otherwise. Both surfaces cancel a job through this.
    pub(crate) fn cancel(&mut self, id: &str) -> Result<(), &'static str> {
        if self.id.as_deref() == Some(id) {
            self.cancellation.cancel();
            return Ok(());
        }
        self.commands
            .try_send(CodingCommand::Cancel { id: id.to_owned() })
            .map_err(|_| CONTROL_FULL)
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }
}
impl Drop for CodingPanel {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
/// The receipt a start shows so the user can name the job later with
/// `/job show <id>`. A non-start control names no new job.
pub(crate) fn started_line(command: &CodingCommand) -> Option<String> {
    match command {
        CodingCommand::Start { id, .. } => Some(format!("Started job {id}")),
        _ => None,
    }
}
