//! The slash commands that leave the surface (`03` §4, A9).
//!
//! A line the surface does not answer itself — `/help`, `/exit`, `/clear`, the
//! context changes — is the session's own operation, and the chat layer states
//! which one each command means ([`ChatOps`]). This module performs that table
//! on the surface's side: the words an operation answered with are said, the
//! one that ends the run asks the loop to leave, and an operation whose own
//! surface has not landed here says so instead of being sent to the model.
//!
//! Nothing here runs a turn: `/sql`, `/export`, `/chart` and `/investigation`
//! are [`super::direct`]'s, and a plain message is the composer's.

use crate::chat::{ChatOps, OpResult, ReadOp, SessionOp};
use crate::slash::user_commands::UserCommands;
use crate::slash::{SlashCommand, help_for};

/// What an operation needs besides the chat layer's own table: the user's
/// commands, which answer for their names in `/help`.
pub(crate) struct OpsDeps {
    pub(crate) chat: ChatOps,
    pub(crate) commands: UserCommands,
}

/// What the surface must do for one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Asked {
    /// Land these words in the transcript as a notice, and nothing else.
    Notice(String),
    /// The same, as the refusal it is.
    Error(String),
    /// Leave: the run is closed and the terminal restored.
    Quit,
}

/// What the surface says for a command whose own surface has not landed here
/// yet. One sentence, so the operator learns the rule once: this release's
/// full-screen surface answers the commands it can, and every other command is
/// still the line front's.
pub(crate) const ELSEWHERE: &str = "that command is the line front's in this release — \
     this surface answers /help, /mode, /job, /exit, /sql, /export, /chart and /investigation";

/// Performs one submitted command as the operation the chat layer states.
pub(crate) fn perform(deps: &OpsDeps, command: SlashCommand) -> Asked {
    match deps.chat.run(command) {
        // The chat layer performed it, and these are the words it answered with.
        OpResult::Done(words) => Asked::Notice(words),
        OpResult::Refused(words) => Asked::Error(words),
        // `/help` is a read of nothing but the catalogue: its words are the
        // session's own command table, in the same text the line front prints.
        OpResult::Read(ReadOp::Help(topic)) => {
            Asked::Notice(help_for(topic.as_deref(), &deps.commands))
        }
        // `/exit` and `/quit`: the one session op that ends the process's
        // surface rather than the run it is on.
        OpResult::Session(SessionOp::Exit) => Asked::Quit,
        OpResult::Unavailable(wording) => Asked::Error(wording),
        // Everything else — a rollover, a read of something other than the
        // run, another session op — belongs to a surface of its own.
        OpResult::Session(_) | OpResult::Rollover(_) | OpResult::Read(_) => {
            Asked::Notice(ELSEWHERE.to_owned())
        }
    }
}
