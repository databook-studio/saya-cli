mod consent;
mod pump;
mod worker;
use super::{transcript::BlockKind, types::App};
use crate::agent::tools::shell_policy::ModeCell;
use crate::cli::CodingCommand;
use saya_agent::{ApprovalChoice, CancellationToken};
use std::{
    sync::{Arc, OnceLock, mpsc},
    thread::JoinHandle,
};
use tokio::sync::oneshot;

#[cfg(test)]
#[path = "panel_tests.rs"]
mod panel_tests;
#[cfg(test)]
mod receipt_tests;

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
    /// The running job's one permission-mode cell, published by the worker once
    /// it has built the job: the handle a mode switch reaches it through.
    job_mode: Arc<OnceLock<ModeCell>>,
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
    /// The job this panel is running, as the switch addresses it: `None` until
    /// the worker has built it, and for a command that runs no root at all.
    pub(crate) fn running_job(&self) -> Option<ModeCell> {
        self.job_mode.get().cloned()
    }
    /// A panel with no worker, holding `job`'s cell: what a live panel looks
    /// like from the switch's side once its worker has built the job.
    #[cfg(test)]
    pub(crate) fn holding(job: ModeCell) -> Self {
        let (_activity, rx) = mpsc::sync_channel(1);
        let (_receipt_tx, receipts) = mpsc::sync_channel(1);
        let (_done_tx, done) = mpsc::sync_channel(1);
        let (commands, _control) = tokio::sync::mpsc::channel(1);
        let job_mode = OnceLock::new();
        let _ = job_mode.set(job);
        Self {
            id: None,
            rx,
            receipts,
            done,
            commands,
            cancellation: CancellationToken::new(),
            pending: None,
            worker: None,
            active: true,
            job_mode: Arc::new(job_mode),
        }
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
impl App {
    /// The job the panel is running, if one is: the cell the session's mode
    /// switch also states a mode on. `None` when nothing runs, so a switch made
    /// with no job under it changes nothing but the session.
    pub(crate) fn running_job(&self) -> Option<ModeCell> {
        self.coding_panel
            .as_ref()
            .filter(|panel| panel.is_active())
            .and_then(CodingPanel::running_job)
    }
    pub(crate) fn coding_command(
        &mut self,
        command: CodingCommand,
        root: Option<&std::path::Path>,
        approval_policy: saya_agent::ApprovalPolicy,
    ) {
        if let Some(panel) = self.coding_panel.as_mut().filter(|panel| panel.active) {
            let result = match &command {
                CodingCommand::Approve { id } | CodingCommand::Deny { id }
                    if panel.id.as_deref() == Some(id) =>
                {
                    panel.pending.take().ok_or(NO_PENDING).map(|reply| {
                        let _ = reply.send(matches!(&command, CodingCommand::Approve { .. }));
                    })
                }
                CodingCommand::Cancel { id } => panel.cancel(id),
                _ => panel.commands.try_send(command).map_err(|_| CONTROL_FULL),
            };
            if let Err(message) = result {
                self.transcript.push(BlockKind::Error, message);
            }
            return;
        }
        let Some(root) = root else {
            self.transcript.push(
                BlockKind::Error,
                "Bind a repository workspace before starting a job.",
            );
            return;
        };
        if let Some(line) = started_line(&command) {
            self.transcript.push(BlockKind::System, line);
        }
        self.coding_panel = Some(worker::spawn(
            command,
            self.runtime.as_ref().clone(),
            self.state_db.clone(),
            root.to_owned(),
            approval_policy,
            // This surface composes no session run of its own, so a job it
            // opens is nobody's dependent and names no session.
            None,
        ));
    }
    pub(crate) fn poll_coding(&mut self) {
        // The panel is taken out of the app for the drain: a job's question is
        // posted onto the app (the chat's approval prompt), and the panel is
        // put back before this returns.
        let Some(mut panel) = self.coding_panel.take() else {
            return;
        };
        let mut questions: Vec<(String, String, oneshot::Sender<ApprovalChoice>)> = Vec::new();
        for _ in 0..32 {
            match panel.rx.try_recv() {
                Ok(CodingMessage::Activity(text)) => {
                    self.transcript.push(BlockKind::System, safe_text(&text))
                }
                Ok(CodingMessage::Approval { text, respond }) => {
                    panel.pending = Some(respond);
                    self.transcript.push(BlockKind::System, safe_text(&text));
                }
                Ok(CodingMessage::Question {
                    subject,
                    target,
                    respond,
                }) => questions.push((subject, target, respond)),
                Err(_) => break,
            }
        }
        // Receipts push last: the frame at the top of the next loop
        // iteration runs before the next drain, so the newest block is
        // inside the viewport and the receipt is painted.
        for _ in 0..32 {
            match panel.receipts.try_recv() {
                Ok(CodingMessage::Activity(text)) => {
                    self.transcript.push(BlockKind::System, safe_text(&text))
                }
                Ok(CodingMessage::Approval { text, respond }) => {
                    panel.pending = Some(respond);
                    self.transcript.push(BlockKind::System, safe_text(&text));
                }
                // A receipt queue carries receipts only: a question never
                // rides it.
                Ok(CodingMessage::Question { .. }) => {}
                Err(_) => break,
            }
        }
        match panel.done.try_recv() {
            Ok(result) => {
                panel.active = false;
                panel.pending.take();
                match result {
                    Ok(text) => self.transcript.push(BlockKind::System, safe_text(&text)),
                    Err(text) => self.transcript.push(BlockKind::Error, safe_text(&text)),
                }
                if let Some(worker) = panel.worker.take() {
                    let _ = worker.join();
                }
            }
            Err(mpsc::TryRecvError::Disconnected) if panel.active => {
                panel.active = false;
                self.transcript.push(BlockKind::Error, WORKER_GONE);
            }
            Err(_) => {}
        }
        self.coding_panel = Some(panel);
        // Posted last: the chat's prompt is single-pending, so a question that
        // arrives mid-drain finds it free rather than replacing an open one.
        for (subject, target, respond) in questions {
            self.post_approval(subject, Some(target), None, respond);
        }
    }
}
fn safe_text(text: &str) -> String {
    saya_types::redact(text)
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(16 * 1024)
        .collect()
}
