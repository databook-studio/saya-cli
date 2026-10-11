mod worker;
use super::{transcript::BlockKind, types::App};
use crate::cli::CodingCommand;
use saya_agent::CancellationToken;
use std::{sync::mpsc, thread::JoinHandle};
use tokio::sync::oneshot;

#[cfg(test)]
mod receipt_tests;

pub(super) enum CodingMessage {
    Activity(String),
    Approval {
        text: String,
        respond: oneshot::Sender<bool>,
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
impl App {
    pub(crate) fn coding_command(
        &mut self,
        command: CodingCommand,
        root: Option<&std::path::Path>,
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
                    |_| "Coding control mailbox is full or closed; cancellation remains available",
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
                "Bind a repository workspace before coding.",
            );
            return;
        };
        self.coding_panel = Some(worker::spawn(
            command,
            self.runtime.as_ref().clone(),
            self.state_db.clone(),
            root.to_owned(),
        ));
    }
    pub(crate) fn poll_coding(&mut self) {
        let Some(panel) = self.coding_panel.as_mut() else {
            return;
        };
        for _ in 0..32 {
            match panel.rx.try_recv() {
                Ok(CodingMessage::Activity(text)) => {
                    self.transcript.push(BlockKind::System, safe_text(&text))
                }
                Ok(CodingMessage::Approval { text, respond }) => {
                    panel.pending = Some(respond);
                    self.transcript.push(BlockKind::System, safe_text(&text));
                }
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
                self.transcript.push(BlockKind::Error,"Coding worker ended without a settled result; inspect recovery before continuing.");
            }
            Err(_) => {}
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
