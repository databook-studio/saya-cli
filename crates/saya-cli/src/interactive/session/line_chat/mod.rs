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
//! **Opened once, prepared before the first message.** The run is composed and
//! opened ahead of the first line that is not a slash command — while the
//! operator is still at the prompt ([`LineChat::prepare`]) — and that line, and
//! every later one, is a turn of the same run: on the lease, epoch and provider
//! it took at open, until a compaction or `/clear` rolls it over, and the line
//! after it is a turn of the new job. A line that is a slash command never
//! reaches this module, and gives up a prepared run it made stale.
//!
//! **One runtime per session.** The run's composition, its lease and its
//! timers belong to the tokio runtime that opened them, so every turn of one
//! session runs on the same one — the line loop's own `block_on` builds a
//! runtime per call, which is fine for one-shot operations and not for a run
//! that outlives them.

mod begin;
mod boundary;
mod line;
mod one_shot;
mod open;
mod resume;
mod run;
mod session_host;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use saya_store::RedactedSession;

use crate::interactive::turn_summary::ToolTally;

pub(crate) use self::open::{LineSession, open_surface};

/// The line session a surface outside the line loop holds: its run, its
/// transcript, its compaction and its rollover.
#[allow(unused_imports)]
pub(crate) use self::line::Line;

#[allow(unused_imports)]
pub(crate) use self::open::Opened;

/// The one composition every line surface passes through: for the surfaces
/// that state a run over a session's parts, and for the tests that read what
/// shape the folder gives the run it opens.
#[allow(unused_imports)]
pub(crate) use self::session_host::open_host;

pub(crate) use self::begin::Begun;

/// The facts about a saved record's linked job, as the line front reads them:
/// what a picker's row is decided from, so the row cannot disagree with the
/// resume that follows it.
#[allow(unused_imports)]
pub(crate) use self::begin::facts as record_facts;

/// The line front's own resume decision, and what it is read over: the one
/// answer the run this process opens and a picker's row both come from.
#[allow(unused_imports)]
pub(crate) use self::resume::{
    Facts as ResumeFacts, decision as resume_decision, facts as resume_facts,
};

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

#[cfg(all(test, unix))]
#[path = "workspace_tests.rs"]
mod workspace_tests;

#[cfg(all(test, unix))]
#[path = "prepared_close_tests.rs"]
mod prepared_close_tests;

#[cfg(test)]
#[path = "open_tests.rs"]
mod open_tests;

#[cfg(test)]
#[path = "open_stall_tests.rs"]
mod open_stall_tests;

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

    /// One turn of one session, for a surface that has no line loop to drive
    /// it: the same composition the plain loop opens on its first message
    /// ([`open`]), the question sent as that session's one turn, and the
    /// session closed. The caller drives it on the runtime it is already on, so
    /// the run lives and ends inside that runtime — `saya ask` and each
    /// candidate attempt are its callers.
    ///
    /// `None` means the message never ran (the session refused it).
    pub(crate) async fn one_turn(
        parts: LineSession,
        message: String,
    ) -> Result<Option<Sent>, String> {
        let mut line = open::one(parts).await?;
        let sent = line.send(message).await;
        line.quit().await;
        sent
    }

    /// The link a saved record carries to the run this session belongs to: a
    /// rollover moves it to the job that follows.
    pub(crate) fn run_link(&self) -> Option<saya_store::RunLink> {
        self.lock().run_link()
    }

    /// The session facts this front's compaction states and its requests are
    /// rendered from: the one cell the loop applies to its own session state.
    pub(crate) fn session_facts(&self) -> Option<Arc<crate::chat::SessionFacts>> {
        self.lock().facts()
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

    /// Ends a prepared run no line adopted — the run a slash command gave up,
    /// and the run a session ends while it prepared: the session's own quit, and
    /// the job's end, so the folder's lease is free again and no job is left
    /// behind for a session that said nothing.
    pub(crate) fn end_prepared_run(&self) {
        let line = self.lock();
        if let Some(run) = line.run() {
            self.runtime.block_on(run.end_prepared_run());
        }
    }

    fn lock(&self) -> MutexGuard<'_, line::Line> {
        self.line.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts composing and opening the session's run on a thread of its own,
    /// before anything has been asked: the operator's typing is spent on the
    /// run the first line is a turn of, never on composing it. The returned
    /// handle is the loop's — it adopts the run when the first plain line
    /// arrives ([`Preparing::adopt`]), and gives it up when a line makes it
    /// stale or when no line ever comes ([`Preparing::abandon`]).
    pub(crate) fn prepare(parts: LineSession, record: RedactedSession) -> Preparing {
        let worker = std::thread::spawn(move || Self::begin(parts, record));
        Preparing {
            worker: Some(worker),
        }
    }
}

/// One word for a prepare that could not be waited for, and one for a session
/// whose prepare never started: the first line states either in its own turn.
const PREPARE_LOST: &str = "the session's run could not be prepared: the prepare ended \
     without answering";
const PREPARE_UNSTARTED: &str = "the session's run could not be prepared: no prepare was \
     started";

/// The run a line session composed ahead of its first line.
pub(crate) struct Preparing {
    worker: Option<std::thread::JoinHandle<Result<Begun, String>>>,
}

impl Preparing {
    /// Waits for the composition and hands the session back: the run it opened,
    /// or the error the first line states exactly as it would have.
    pub(crate) fn adopt(mut self) -> Result<Begun, String> {
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .unwrap_or_else(|_| Err(PREPARE_LOST.to_owned())),
            None => Err(PREPARE_UNSTARTED.to_owned()),
        }
    }

    /// Ends a prepare no line adopted — a slash command made the run stale, or
    /// no line ever came: the run it composed is closed and its job ended, so
    /// nothing is left working, no folder is left held and no job is left
    /// recorded for a session that has said nothing.
    pub(crate) fn abandon(mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        if let Ok(Begun::Open { chat, .. }) = worker.join().unwrap_or_else(|_| {
            Ok(Begun::Refused {
                sentence: PREPARE_LOST.to_owned(),
            })
        }) {
            chat.end_prepared_run();
        }
    }
}
