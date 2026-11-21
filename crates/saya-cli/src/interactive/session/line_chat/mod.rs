//! The plain REPL's chat session: one run composed and prepared once, every
//! message a turn of it.
//!
//! The session itself is the chat layer's ([`crate::chat`]): this module is
//! only the glue that builds what a line surface resolved — the folder, the
//! connections, the mode, the privacy and memory facts — and keeps the run
//! alive between lines.
//!
//! **Opened once, prepared before the first message.** The first line that is
//! not a slash command composes the run and opens it ([`open`]); every later
//! line is a turn of that same run, on the lease, epoch and provider it took at
//! open. A line that is a slash command never reaches this module.
//!
//! **One runtime per session.** The run's composition, its lease and its
//! timers belong to the tokio runtime that opened them, so every turn of one
//! session runs on the same one — the line loop's own `block_on` builds a
//! runtime per call, which is fine for one-shot operations and not for a run
//! that outlives them.

mod open;

use std::sync::Arc;

use saya_agent::CancellationToken;
use saya_store::RunLink;

use crate::chat::{ChatOutcome, ChatSession, RunComposition};
use crate::commands::coding::{CodingApplication, NativeCodingJob};
use crate::front::LineFront;
use crate::interactive::turn_summary::ToolTally;

pub(crate) use self::open::LineSession;

/// One line session's open run.
pub(crate) struct LineChat {
    /// The host the run was composed over. Held so the job's own `Arc` has an
    /// owner above it, and so a later unit's rollover composes over the same
    /// one.
    _application: Arc<CodingApplication>,
    /// The run as it was composed and opened: its mode cell and its operator.
    _run: RunComposition,
    session: ChatSession,
    front: Arc<LineFront>,
    job: Arc<NativeCodingJob>,
    /// The runtime that opened the run: every turn of it runs here.
    runtime: tokio::runtime::Runtime,
}

impl LineChat {
    /// The link a saved record carries to the run this session belongs to.
    pub(crate) fn run_link(&self) -> Option<RunLink> {
        let job = self.job.job_id().clone();
        // Through the chat layer's own writer, so there is one shape of link.
        let mut record = saya_store::RedactedSession::default();
        crate::chat::after_prepare(&mut record, &job, self.job.epoch());
        record.run
    }

    /// Sends one operator line as a turn of the session's run, and reports how
    /// it settled. `None` means the message never ran (the session refused it:
    /// it is closing, or a turn it queued behind never ended).
    ///
    /// A Ctrl-C during the turn ends that turn — the question it left pending
    /// with it — and leaves the run taking the next line, which is today's
    /// behaviour. The rest of the key handling belongs to the next unit.
    pub(crate) fn send(&self, message: String) -> Result<Option<ChatOutcome>, String> {
        let turn = CancellationToken::new();
        self.front.set_turn(turn.clone());
        self.runtime.block_on(async {
            let send = self.session.send(message);
            tokio::pin!(send);
            tokio::select! {
                _ = &mut send => {}
                _ = tokio::signal::ctrl_c() => {
                    // The turn's own stop and the open question's: the message
                    // does not run, and the run stays open for the next line.
                    turn.cancel();
                    self.session.cancel().await;
                    send.await;
                }
            }
        });
        Ok(self.front.take_outcome())
    }

    /// The tool tally of every turn this session ran, for the scripted run's
    /// one terminal result.
    pub(crate) fn tally(&self) -> ToolTally {
        self.front.tally()
    }

    /// Ends the run: the session's own quit, stated once. A turn in flight is
    /// not abandoned (the loop cancels it first); a quit while one runs is
    /// refused and the run stays open, which no later line reaches.
    pub(crate) fn close(&self) {
        let _ = self.runtime.block_on(self.session.quit());
    }
}
