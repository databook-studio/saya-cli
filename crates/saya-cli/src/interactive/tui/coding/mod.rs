mod worker;
use super::{transcript::BlockKind, types::App};
use crate::cli::CodingCommand;
use saya_agent::{ApprovalChoice, CancellationToken};
use std::{sync::mpsc, thread::JoinHandle};
use tokio::sync::oneshot;

#[cfg(test)]
#[path = "panel_tests.rs"]
mod panel_tests;
#[cfg(test)]
mod receipt_tests;

pub(super) enum CodingMessage {
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
    /// Control receipts (queued / pause / continue / cancel outcomes and
    /// pause acknowledgements) on their own bounded queue: `poll_coding`
    /// drains activity first and pushes receipts last, so a receipt is the
    /// newest block of its tick and the next frame — which runs before the
    /// next drain — always paints it. Sharing the activity queue let a
    /// same-tick burst of progress events scroll a receipt above the
    /// viewport before any frame drew it.
    receipts: mpsc::Receiver<CodingMessage>,
    done: mpsc::Receiver<Result<String, String>>,
    commands: tokio::sync::mpsc::Sender<CodingCommand>,
    cancellation: CancellationToken,
    pending: Option<oneshot::Sender<bool>>,
    worker: Option<JoinHandle<()>>,
    active: bool,
}
impl CodingPanel {
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
fn started_line(command: &CodingCommand) -> Option<String> {
    match command {
        CodingCommand::Start { id, .. } => Some(format!("Started job {id}")),
        _ => None,
    }
}
impl App {
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
                    panel
                        .pending
                        .take()
                        .ok_or("No pending approval for that job")
                        .map(|reply| {
                            let _ = reply.send(matches!(&command, CodingCommand::Approve { .. }));
                        })
                }
                CodingCommand::Cancel { id } if panel.id.as_deref() == Some(id) => {
                    panel.cancellation.cancel();
                    Ok(())
                }
                _ => panel.commands.try_send(command).map_err(
                    |_| "Job control mailbox is full or closed; cancellation remains available",
                ),
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
                self.transcript.push(BlockKind::Error,"Job worker ended without a settled result; inspect recovery before continuing.");
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
