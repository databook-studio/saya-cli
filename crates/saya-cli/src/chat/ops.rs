//! One session operation per slash command.
//!
//! A slash command is parsed once ([`crate::slash`]) and then means exactly one
//! thing to a chat session. This module is that one thing: [`ChatOps::operation`]
//! is an exhaustive `match`, so a new command variant cannot arrive without a
//! stated fate, and every fate says who performs the operation:
//!
//! * [`Fate::Session`] — the session's own state, never a turn of the run;
//!   bare `/continue` is the exception that states one: the session's adapter
//!   sends the literal `Continue.` through the typed line's own path
//!   (`chat::ops_continue`), and this layer never enters the port to do it.
//! * [`Fate::Rollover`] — a context change, which only ever *requests* the
//!   rollover; the session performs it;
//! * [`Fate::ReadOnly`] — a shared operation on something other than the run;
//! * [`Fate::Unavailable`] — not available in chat yet, in today's words.
//!
//! Nothing here renders or prompts. A result is a typed value a front projects,
//! and the run's port is in hand only so the guarantee above is visible where a
//! command is dispatched: no operation in this table enters the run, and the
//! tests assert the port sees no call. The one command that states a turn
//! (`/continue` bare) states it as a [`SessionOp`], for the adapter that owns
//! the session to send.

use std::sync::Arc;

use super::RunPort;
use super::ops_context::{ContextChange, OpsContext, ReadOp, RolloverRequest, SessionOp};
use super::ops_status::Section;
use crate::interactive::session::permissions::mode;
use crate::slash::SlashCommand;

/// The wording a `/run` in a chat session gets: the run engine is a separate
/// engine, and a chat session does not drive it.
const RUN_ELSEWHERE: &str = "`/run` drives the separate `saya run` engine, which a chat session \
     does not: run `saya run` from your shell.";

/// The class of operation one command maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fate {
    /// An operation on the session itself.
    Session,
    /// A context change that requests a rollover.
    Rollover,
    /// A shared operation on something other than the run.
    ReadOnly,
    /// Not available in chat yet.
    Unavailable,
}

/// The one operation a command maps to, with everything it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    /// A session op, and who performs it.
    Session(SessionOp),
    /// A context change, and the value the command stated.
    Rollover(ContextChange, Option<String>),
    /// A shared operation to run outside the run's turns.
    Read(ReadOp),
    /// Not available in chat yet, with the refusal's own wording.
    Unavailable(&'static str),
}

impl Op {
    /// Which of the four fates this operation belongs to.
    pub(crate) fn fate(&self) -> Fate {
        match self {
            Self::Session(_) => Fate::Session,
            Self::Rollover(_, _) => Fate::Rollover,
            Self::Read(_) => Fate::ReadOnly,
            Self::Unavailable(_) => Fate::Unavailable,
        }
    }
}

/// What one command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpResult {
    /// The chat layer performed the operation; these are the words the
    /// operator reads.
    Done(String),
    /// Refused, in today's words. Nothing changed.
    Refused(String),
    /// A session op for the adapter that owns that state.
    Session(SessionOp),
    /// The session must roll over. Never performed here.
    Rollover(RolloverRequest),
    /// The shared operation to run outside the run's turns.
    Read(ReadOp),
    /// Not available in chat yet, in today's words.
    Unavailable(String),
}

/// The one operation each slash command means to a chat session.
pub(crate) struct ChatOps {
    /// The run behind the session. Held, never entered: every operation in
    /// today's table stays outside the run's turns, and the tests assert the
    /// port sees no call. An operation that must reach the run would have to
    /// say so here.
    run: Arc<dyn RunPort>,
    context: OpsContext,
}

impl ChatOps {
    /// Ops over `run`, touching only what `context` states.
    pub(crate) fn new(run: Arc<dyn RunPort>, context: OpsContext) -> Self {
        Self { run, context }
    }

    /// The one operation `command` maps to.
    pub(crate) fn operation(&self, command: &SlashCommand) -> Op {
        match command {
            // The job surface: a separate job the session starts or steers,
            // never a turn of the session's own run.
            SlashCommand::Coding(command) => Op::Session(SessionOp::Job(command.clone())),
            // A context change: the database, the model, the provider, privacy.
            SlashCommand::Connect(profile) => {
                Op::Rollover(ContextChange::Connect, Some(profile.clone()))
            }
            SlashCommand::Include(profile) => {
                Op::Rollover(ContextChange::Include, Some(profile.clone()))
            }
            SlashCommand::Exclude(profile) => {
                Op::Rollover(ContextChange::Exclude, Some(profile.clone()))
            }
            // Bare shows what is in force; a stated value changes the context.
            SlashCommand::Provider(None) => Op::Read(ReadOp::Provider),
            SlashCommand::Provider(Some(name)) => {
                Op::Rollover(ContextChange::Provider, Some(name.clone()))
            }
            SlashCommand::Model(None) => Op::Read(ReadOp::Model),
            SlashCommand::Model(Some(name)) => {
                Op::Rollover(ContextChange::Model, Some(name.clone()))
            }
            SlashCommand::Privacy(None) => Op::Read(ReadOp::Privacy),
            SlashCommand::Privacy(Some(sharing)) => Op::Rollover(
                ContextChange::Privacy,
                Some(if *sharing { "on" } else { "off" }.to_owned()),
            ),
            SlashCommand::Connections => Op::Read(ReadOp::Connections),
            SlashCommand::Mode(request) => Op::Session(SessionOp::Mode(*request)),
            SlashCommand::Schema(refresh) => Op::Read(ReadOp::Schema { refresh: *refresh }),
            SlashCommand::Sql(query) => Op::Read(ReadOp::Sql(query.clone())),
            SlashCommand::Export(request) => Op::Read(ReadOp::Export(request.clone())),
            SlashCommand::Chart(kind) => Op::Read(ReadOp::Chart(kind.clone())),
            SlashCommand::Explain(sql) => Op::Read(ReadOp::Explain(sql.clone())),
            // The conversation itself: emptying or shrinking it is a new epoch.
            SlashCommand::Clear => Op::Rollover(ContextChange::Clear, None),
            // One compaction operation: bare shrinks this session's working
            // memory, `--new-session` (and its one-release alias `/rotate`)
            // continues in a successor afterwards.
            SlashCommand::Compact(request) => Op::Rollover(
                if request.new_session {
                    ContextChange::Rotate
                } else {
                    ContextChange::Compact
                },
                None,
            ),
            SlashCommand::Resume(id) => Op::Rollover(ContextChange::Resume, Some(id.clone())),
            // Everything else reads or maintains something other than the run.
            SlashCommand::Sessions(_) => Op::Read(ReadOp::Sessions),
            SlashCommand::SessionsList => Op::Read(ReadOp::SessionsList),
            SlashCommand::SessionsDelete(id) => Op::Read(ReadOp::SessionsDelete(id.clone())),
            SlashCommand::Preferences(command) => Op::Read(ReadOp::Preferences(command.clone())),
            SlashCommand::Contracts(request) => Op::Read(ReadOp::Contracts(request.clone())),
            SlashCommand::Investigation(command) => {
                Op::Read(ReadOp::Investigation(command.clone()))
            }
            SlashCommand::Runs(id) => Op::Read(ReadOp::Runs(id.clone())),
            // The run engine is separate, and a chat session does not drive it.
            SlashCommand::Run(_) | SlashCommand::RunCancel(_) => Op::Unavailable(RUN_ELSEWHERE),
            SlashCommand::Allow(tokens) => Op::Session(SessionOp::Allow(tokens.clone())),
            // One read-only view of the session's permissions, and the alias
            // that states the same read.
            SlashCommand::Permissions(request) => Op::Read(ReadOp::Permissions(*request)),
            // The one panel, and the three aliases that open one of its
            // sections: one operation, one set of facts, one rendering.
            SlashCommand::Status => Op::Read(ReadOp::Status),
            SlashCommand::Usage => Op::Read(ReadOp::StatusSection(Section::Model)),
            SlashCommand::Workspace => Op::Read(ReadOp::StatusSection(Section::Workspace)),
            SlashCommand::Context => Op::Read(ReadOp::StatusSection(Section::Instructions)),
            SlashCommand::Doctor => Op::Read(ReadOp::Doctor),
            SlashCommand::Thinking(shown) => Op::Session(SessionOp::Thinking(*shown)),
            SlashCommand::Help(name) => Op::Read(ReadOp::Help(name.clone())),
            SlashCommand::Tasks(tail) => Op::Read(ReadOp::Tasks(tail.clone())),
            SlashCommand::Notes(command) => Op::Read(ReadOp::Notes(command.clone())),
            SlashCommand::Recovery(command) => Op::Read(ReadOp::Recovery(command.clone())),
            // The run's own checkpoints: the session's to restore, never a turn
            // of the run, so both are session ops and neither runs mid-turn.
            SlashCommand::Undo => Op::Session(SessionOp::Undo),
            SlashCommand::Rewind(request) => Op::Session(SessionOp::Rewind(request.clone())),
            // Continue a paused turn: a session op that states the one message,
            // sent by the adapter through the typed line's own path.
            SlashCommand::ContinueTurn => Op::Session(SessionOp::ContinueTurn),
            SlashCommand::Setup => Op::Session(SessionOp::Setup),
            SlashCommand::Reload => Op::Session(SessionOp::Reload),
            SlashCommand::Columns(names) => Op::Session(SessionOp::Columns(names.clone())),
            SlashCommand::Exit => Op::Session(SessionOp::Exit),
        }
    }

    /// Performs the operation `command` maps to.
    ///
    /// Only the operations the chat layer owns run here: a mode stated on the
    /// run's cell, and the words a bare `/mode` reads back. Everything else is
    /// handed on as the typed value it is — a rollover request for the session,
    /// a shared read operation for the surface, a session op for the adapter
    /// that owns that state.
    pub(crate) fn run(&self, command: SlashCommand) -> OpResult {
        match self.operation(&command) {
            Op::Session(SessionOp::Mode(request)) => match request.value {
                None => OpResult::Done(self.context.mode_words()),
                // The alias's deprecation note rides the one switch's answer,
                // so `/approvals` and `/mode` cannot answer differently about
                // the mode itself.
                Some(next) => match mode::with_alias_note(self.context.state_mode(next), request) {
                    Ok(message) => OpResult::Done(message),
                    Err(refusal) => OpResult::Refused(refusal),
                },
            },
            Op::Session(op) => OpResult::Session(op),
            Op::Rollover(change, value) => OpResult::Rollover(RolloverRequest { change, value }),
            Op::Read(op) => OpResult::Read(op),
            Op::Unavailable(wording) => OpResult::Unavailable(wording.to_owned()),
        }
    }
}

#[cfg(test)]
#[path = "ops_fixture.rs"]
mod fixture;

#[cfg(test)]
#[path = "ops_fates_tests.rs"]
mod fates_tests;

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
