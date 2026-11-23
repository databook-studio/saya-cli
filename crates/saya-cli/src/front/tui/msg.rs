//! What reaches the model, and what the model asks the runtime to do.
//!
//! Two directions, two types: [`Msg`] is everything the surface reacts to —
//! one input thread's keys, a paste, a resize, a session event, the tick —
//! and [`Cmd`] is the side effect the pure [`update`](super::update) decided
//! on but must not perform itself. The runtime between them is what actually
//! sends, cancels, answers and switches.

use ratatui::crossterm::event::KeyEvent;
use saya_agent::PermissionMode;

use super::direct::{Direct, DirectOutcome};
use super::jobs::JobReport;
use crate::agent::tools::consent::Answer;
use crate::chat::ChatEvent;

/// What a background worker reports back.
///
/// A worker that produced something hands it to the unit that asked for it;
/// what the surface has to say for itself — a worker that could not finish —
/// is what lands here. A struct rather than a bare sentence, so the units that
/// add a field do not have to rewrite every construction of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Worker {
    /// The worker's own words about itself, said to the operator.
    pub(crate) message: String,
    /// What a direct command handed back, when it handed something back: the
    /// rows a query returned, or the sentence a command ends in. The units
    /// that ask a worker for something else add their field beside it.
    pub(crate) direct: Option<DirectOutcome>,
    /// What a job's plumbing said: the run's job list as it now is, or one call
    /// a job puts to the operator (`02` §8). It rides the worker slot because a
    /// job is a worker of this surface — never the queue's key slot, so a slow
    /// job can never read as a press.
    pub(crate) job: Option<JobReport>,
}

/// One thing the surface has to react to.
#[derive(Debug)]
pub(crate) enum Msg {
    /// One key press, as the input thread read it.
    Key(KeyEvent),
    /// One bracketed paste, as one event: a pasted block never reads as
    /// keystrokes, so a `\r` in it cannot send it.
    Paste(String),
    /// The terminal's new size. Nothing in the model reads it yet: the panes
    /// lay themselves out from the frame they are given.
    Resize { width: u16, height: u16 },
    /// One session event, passed on unchanged.
    Chat(ChatEvent),
    /// Whether the terminal window has the operator's focus. `true` when it
    /// gained it, `false` when it lost it: a turn's end is only news while
    /// the operator is looking elsewhere, when the terminal says so at all.
    Focus(bool),
    /// The paths the run's workspace offers, as the walk behind the `@` list
    /// returned them (`Cmd::Mentions`). They arrive on the queue like every
    /// other answer the surface waits for, so a slow walk never holds a frame.
    Mentions(Vec<String>),
    /// One background worker's answer. The panes that ask a worker — the
    /// result table, an export, a chart, a compaction, the clipboard — land in
    /// their own units; what this queue needs from the start is that a
    /// worker's answer has a place of its own, so a slow one can never read as
    /// a key press.
    Worker(Worker),
    /// The 250 ms tick that drives the spinner and expires a toast.
    Tick,
    /// The surface is leaving: the run is closed and the terminal restored
    /// (`/exit`, `/quit`). The loop ends on it, after painting nothing more.
    Quit,
    /// The session's own facts, restated by a command that changed them
    /// (`/model`, `/provider`, `/connect`): the status row names what the
    /// session is on now, never the model and database the launch resolved.
    Restated {
        /// The model the session states, as the launch states its own.
        model: String,
        /// The database the session is connected to, if it is.
        database: Option<String>,
    },
}

/// One side effect the runtime performs after an [`update`](super::update).
///
/// The model never performs these itself: that is what keeps a whole session
/// — sending, queueing, cancelling, answering — testable without a terminal.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cmd {
    /// Run one message as a turn.
    Send(String),
    /// Replace the last turn: the previous turn's file changes are rewound
    /// through the run's own checkpoints, then this message runs. The
    /// confirmation line says what the rewind changed.
    ReplaceLastTurn(String),
    /// Cancel the turn in flight; the run stays open.
    CancelTurn,
    /// Answer the call asked under this id.
    Answer(u64, Answer),
    /// State a mode on the run's cell.
    SwitchMode(PermissionMode),
    /// State the mode an operator chose in the picker (`02` §8). Performers
    /// route it to the same switch as [`Cmd::SwitchMode`]: the cycle and the
    /// picker differ only in how the mode was chosen.
    SetMode(PermissionMode),
    /// Put this text on the clipboard (`02` §6's `c`): the entry's source, as
    /// the view model holds it, never its painted rows.
    Copy(String),
    /// Run one command the surface answers itself (`03` §4.1): `/sql`,
    /// `/export`, `/chart`, `/investigation`. Performed off the loop's thread;
    /// its answer comes back as [`Msg::Worker`].
    Direct(Direct),
    /// Bind the launch folder as this session's workspace (`02` §9.4): the
    /// trust question's `y`, carried out through today's trust store, exactly
    /// like an explicit `--workspace`.
    Trust,
    /// Cancel the job this id names, through today's job plumbing (`02` §8's
    /// `x`, once the confirm has been answered).
    CancelJob(String),
    /// One job verb of this run (`02` §8): `/start` opens the run's one job
    /// through today's worker, and the rest steer the job it opened. The
    /// consent a start asks for is the card's own question — `/approve` and
    /// `/deny` are that card's answers, never a verb of their own.
    Job(crate::slash::JobRequest),
    /// One line the surface does not answer itself: a session operation, which
    /// the chat layer's own table names ([`super::ops`]).
    Op(crate::slash::SlashCommand),
    /// Gather the paths the `@` list offers: the run's workspace, walked the
    /// way the file tools walk it, off the loop's thread. The answer comes back
    /// as [`Msg::Mentions`].
    Mentions,
    /// Ring the terminal's bell once: a call took the card, and the operator
    /// may be looking elsewhere.
    Bell,
}
