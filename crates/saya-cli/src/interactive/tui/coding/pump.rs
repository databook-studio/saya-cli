//! The panel's own side of a surface's frame: what a job's worker has said
//! since the last look, one control reaching it, and the end it settled with.
//!
//! Today's panel ([`CodingPanel`]) is fed by the job's worker thread through
//! three bounded queues — its progress, its control receipts, and its one
//! result — and the old TUI drains them on its own tick (`App::poll_coding`).
//! A surface of its own looks here instead, so one panel serves whichever
//! surface is live and neither surface reads the worker's channels itself.

use std::sync::mpsc;

use super::{CONTROL_FULL, WORKER_GONE};
use super::{CodingMessage, CodingPanel};
use crate::cli::CodingCommand;

/// The most messages one look takes from a queue: a burst is bounded per look,
/// as today's drain bounds it.
const BURST: usize = 32;

/// What a job's worker has said since the last look, oldest first.
#[derive(Default)]
pub(crate) struct Said {
    /// Progress events: the job's own words about itself, which the drawer
    /// lists as its row.
    pub(crate) activity: Vec<CodingMessage>,
    /// Control receipts and pause acknowledgements: the operator's own lines,
    /// in the worker's words.
    pub(crate) receipts: Vec<CodingMessage>,
}

impl CodingPanel {
    /// The job this panel runs, as a surface addresses it: `None` until the
    /// worker has built it, and for a command that runs no root at all.
    pub(crate) fn job_id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// One control on the running job, in today's words when the mailbox
    /// cannot take it: cancellation stays available either way.
    pub(crate) fn control(&mut self, command: CodingCommand) -> Result<(), &'static str> {
        match command {
            CodingCommand::Cancel { id } => self.cancel(&id),
            command => self.commands.try_send(command).map_err(|_| CONTROL_FULL),
        }
    }

    /// Everything the worker has said since the last look.
    pub(crate) fn drain(&mut self) -> Said {
        let mut said = Said::default();
        for _ in 0..BURST {
            match self.rx.try_recv() {
                Ok(message) => said.activity.push(message),
                Err(_) => break,
            }
        }
        // Receipts last, as today's drain takes them last: the newest block of
        // the look is the receipt, so the next frame paints it.
        for _ in 0..BURST {
            match self.receipts.try_recv() {
                // A receipt queue carries receipts only: a call never rides it.
                Ok(CodingMessage::Question { .. }) => {}
                Ok(message) => said.receipts.push(message),
                Err(_) => break,
            }
        }
        said
    }

    /// The job's own end, on the first look after it ended: the words it
    /// settled with, and its worker joined. `None` while it still runs.
    pub(crate) fn settle(&mut self) -> Option<Result<String, String>> {
        match self.done.try_recv() {
            Ok(result) => {
                self.active = false;
                self.pending.take();
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                Some(result)
            }
            // A worker that went away without a word is a failure, not a
            // silence: the operator is told what to inspect.
            Err(mpsc::TryRecvError::Disconnected) if self.active => {
                self.active = false;
                self.pending.take();
                Some(Err(WORKER_GONE.to_owned()))
            }
            Err(_) => None,
        }
    }
}

/// The panel a test drives, its two report queues, and its one result.
#[cfg(test)]
pub(crate) type Reporting = (
    CodingPanel,
    mpsc::SyncSender<CodingMessage>,
    mpsc::SyncSender<CodingMessage>,
    mpsc::SyncSender<Result<String, String>>,
);

#[cfg(test)]
impl CodingPanel {
    /// A panel whose worker is a test's own channels: an id to be addressed by,
    /// the two queues a worker reports on, and the one result it settles with.
    pub(crate) fn reporting(id: &str) -> Reporting {
        let (activity, rx) = mpsc::sync_channel(8);
        let (receipts_tx, receipts) = mpsc::sync_channel(8);
        let (done_tx, done) = mpsc::sync_channel(1);
        let (commands, _control) = tokio::sync::mpsc::channel(8);
        (
            Self {
                id: Some(id.to_owned()),
                rx,
                receipts,
                done,
                commands,
                cancellation: saya_agent::CancellationToken::new(),
                pending: None,
                worker: None,
                active: true,
            },
            activity,
            receipts_tx,
            done_tx,
        )
    }
}
