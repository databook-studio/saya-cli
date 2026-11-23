//! The slash commands the surface answers (A9, `03` §4).
//!
//! A line the surface does not run as a turn — `/help`, `/exit`, `/status`, a
//! context change, a read of something other than the run — is the session's
//! own operation, and the chat layer states which one each command means
//! ([`ChatOps`]). This module performs that table on the surface's side.
//!
//! **One table, every command.** The full-screen surface is the default one and
//! its palette offers the whole catalogue, so a command that answered "that is
//! another surface's" was a command the operator could choose and could not
//! use. Every operation therefore has a home here: the ones the session-command
//! layer decides are performed through it ([`SessionState::apply`], the same
//! entry the line front's loop applies), the ones that read or write something
//! the launch holds are [`Asked::Work`] for the port to run, and the ones the
//! chat layer states as a rollover are [`Asked::Roll`]. Nothing is re-implemented
//! here: each arm names the shared operation that owns the behaviour.
//!
//! Nothing here runs a turn. `/sql`, `/export`, `/chart` and `/investigation`
//! are [`super::direct`]'s — that reader holds the surface's last result, so it
//! claims them before this table; the arms here answer for a caller with no
//! result in hand. `/setup` is the loop's (it needs the terminal, which this
//! table never touches) and `/reload` is [`Asked::Reload`], performed by the
//! port with the launch's own options ([`super::reload`]).

pub(crate) use super::ops_deps::OpsDeps;
use super::ops_group::{apply, connections_words, read, roll};
use super::ops_work::Work;
use crate::chat::{OpResult, ReadOp, RolloverRequest, SessionOp};
use crate::slash::{SlashCommand, help_for};

pub(crate) use super::ops_deps::OpsParts;

/// What the surface must do for one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Asked {
    /// Land these words in the transcript as a notice, and nothing else.
    Notice(String),
    /// The same, as the refusal it is.
    Error(String),
    /// Leave: the run is closed and the terminal restored.
    Quit,
    /// Re-read the configuration and swap the runtime (`/reload`, and the apply
    /// a closed guided setup runs). The port performs it: it holds the launch's
    /// options and the configuration in force.
    Reload,
    /// Hand the terminal to the guided setup. The loop intercepts `/setup`
    /// before this table (the terminal is the loop's) and performs the handoff;
    /// the variant is here so the table is total and the catalogue guard can see
    /// the command has a home.
    Setup,
    /// Open the job this request states (`/start`, `/job …`). The surface's own
    /// command reader claims the job spellings first; the variant answers for a
    /// caller that reached the table directly.
    Job(crate::slash::JobRequest),
    /// `/continue` — send the paused turn's continuation message. The one
    /// session op that states a turn: the port sends it through the chat
    /// session's own path, and it refuses while a turn runs.
    Continue,
    /// A context change the chat layer's rollover performs, and what the
    /// session's own command layer said about it.
    Roll {
        request: RolloverRequest,
        said: String,
    },
    /// One shared operation the port runs off the loop's thread
    /// ([`super::ops_work`]) and lands as a transcript entry.
    Work(Work),
}

/// Performs one submitted command as the operation the chat layer states.
pub(crate) fn perform(deps: &OpsDeps, command: SlashCommand) -> Asked {
    let asked = deps.chat().run(command.clone());
    match asked {
        OpResult::Done(words) => Asked::Notice(words),
        OpResult::Refused(words) => Asked::Error(words),
        OpResult::Unavailable(wording) => Asked::Error(wording),
        // The one session op that ends the process's surface rather than the
        // run it is on.
        OpResult::Session(SessionOp::Exit) => Asked::Quit,
        // The surface's own re-read, performed by the port with the launch's
        // options — the same apply a closed guided setup runs.
        OpResult::Session(SessionOp::Reload) => Asked::Reload,
        OpResult::Session(SessionOp::Setup) => Asked::Setup,
        OpResult::Session(SessionOp::Job(request)) => Asked::Job(request),
        // `/continue` is the one session op that states a turn: the adapter
        // that owns the session sends the paused turn's message.
        OpResult::Session(SessionOp::ContinueTurn) => Asked::Continue,
        // `/allow` seeds the session's one grant store, through the shared
        // behaviour the line front's loop runs.
        OpResult::Session(SessionOp::Allow(tokens)) => Asked::Work(Work::Allow(tokens)),
        OpResult::Session(_) => apply(deps, command),
        // `/help` is a read of nothing but the catalogue: its words are the
        // session's own command table, in the same text the line front prints.
        OpResult::Read(ReadOp::Help(topic)) => {
            Asked::Notice(help_for(topic.as_deref(), &deps.commands))
        }
        // `/connections`: a read of the configuration in force, so the answer
        // follows a reload instead of repeating what the launch resolved.
        OpResult::Read(ReadOp::Connections) => {
            Asked::Notice(connections_words(&deps.runtime.current()))
        }
        OpResult::Read(op) => read(deps, command, op),
        OpResult::Rollover(request) => roll(deps, command, request),
    }
}
