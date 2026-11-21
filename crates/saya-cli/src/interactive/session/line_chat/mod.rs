//! The plain REPL's chat session: one run composed and prepared once, every
//! message a turn of it — and every turn's boundary compaction's.
//!
//! The session itself is the chat layer's ([`crate::chat`]): this module is
//! only the glue that builds what a line surface resolved — the folder, the
//! connections, the mode, the privacy and memory facts — and keeps the run
//! alive between lines. What a turn does to the session's working memory is
//! [`line`]'s, over [`boundary`]: the chat layer's own compaction, rolled
//! through the chat layer's own rollover, and reached by `/compact` and
//! `/clear` through the same operation.
//!
//! **Opened once, prepared before the first message.** The first line that is
//! not a slash command composes the run and opens it ([`open`]); every later
//! line is a turn of that same run, on the lease, epoch and provider it took at
//! open — until a compaction or `/clear` rolls it over, and the line after it
//! is a turn of the new job. A line that is a slash command never reaches this
//! module.
//!
//! **One runtime per session.** The run's composition, its lease and its
//! timers belong to the tokio runtime that opened them, so every turn of one
//! session runs on the same one — the line loop's own `block_on` builds a
//! runtime per call, which is fine for one-shot operations and not for a run
//! that outlives them.

mod boundary;
mod line;
mod open;
mod run;

use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::interactive::turn_summary::ToolTally;

pub(crate) use self::open::LineSession;

#[allow(unused_imports)]
pub(crate) use self::boundary::{Said, nothing_to_compact};

#[allow(unused_imports)]
pub(crate) use self::run::Sent;

#[cfg(all(test, unix))]
#[path = "boundary_tests.rs"]
mod boundary_tests;

#[cfg(all(test, unix))]
#[path = "boundary_fixture.rs"]
mod boundary_fixture;

/// One line session's open run.
pub(crate) struct LineChat {
    /// The run, its transcript and its compaction, behind one lock: a rollover
    /// replaces the run a line is being sent to.
    line: Mutex<line::Line>,
    /// The runtime that opened the run: every turn of it runs here.
    runtime: tokio::runtime::Runtime,
}

impl LineChat {
    pub(in crate::interactive::session::line_chat) fn new(
        line: line::Line,
        runtime: tokio::runtime::Runtime,
    ) -> Self {
        Self {
            line: Mutex::new(line),
            runtime,
        }
    }

    /// The link a saved record carries to the run this session belongs to: a
    /// rollover moves it to the job that follows.
    pub(crate) fn run_link(&self) -> Option<saya_store::RunLink> {
        self.lock().run_link()
    }

    /// Sends one operator line as a turn of the session's run, and reports how
    /// it settled, what its stream reported, and what its boundary said.
    /// `None` means the message never ran (the session refused it: it is
    /// closing, or a turn it queued behind never ended).
    ///
    /// A Ctrl-C during the turn ends that turn — the question it left pending
    /// with it — and leaves the run taking the next line: only the turn's child
    /// token is cancelled, so the next line is a fresh turn of the same epoch,
    /// on the lease the run already holds.
    pub(crate) fn send(&self, message: String) -> Result<Option<Sent>, String> {
        let mut line = self.lock();
        self.runtime.block_on(line.send(message))
    }

    /// `/compact`: one compaction now, through the same operation the turn's
    /// boundary runs. A conversation with no run open has nothing to compact.
    pub(crate) fn compact(&self) -> Said {
        let mut line = self.lock();
        self.runtime.block_on(line.compact())
    }

    /// `/clear`: the conversation rolls away — the compaction's latch re-armed
    /// with it — through the same clear the chat layer states.
    pub(crate) fn clear(&self) -> Said {
        let mut line = self.lock();
        self.runtime.block_on(line.clear())
    }

    /// The tool tally of every turn this session ran, for the scripted run's
    /// one terminal result.
    pub(crate) fn tally(&self) -> ToolTally {
        self.lock().tally()
    }

    /// Ends the run: the session's own quit, stated once. A turn in flight is
    /// not abandoned (the loop cancels it first); a quit while one runs is
    /// refused and the run stays open, which no later line reaches.
    pub(crate) fn close(&self) {
        let line = self.lock();
        self.runtime.block_on(line.quit());
    }

    fn lock(&self) -> MutexGuard<'_, line::Line> {
        self.line.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
