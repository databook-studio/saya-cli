//! The three families one command falls into, and the shared operation each
//! names: a read of something other than the run, a context change, and a
//! command the session-command layer performs on the session's own state.

use super::ops::Asked;
use super::ops_deps::OpsDeps;
use super::ops_work::Work;
use crate::chat::{ContextChange, ReadOp, RolloverRequest};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::commands::SessionAction;
use crate::slash::SlashCommand;

/// What `/connections` says with no profile configured, in today's words.
pub(crate) const NO_PROFILES: &str = "No configured connection profiles. Add one to \
     .saya/connections.toml (see `saya config init`) or pass --connections.";

/// A read of something other than the run: the session-command layer's, or the
/// shared operation that owns it.
pub(super) fn read(deps: &OpsDeps, command: SlashCommand, read: ReadOp) -> Asked {
    match read {
        // The three bare context reads, and the two the session's own record
        // holds: the state layer answers each in its own words.
        ReadOp::Provider
        | ReadOp::Model
        | ReadOp::Privacy
        | ReadOp::Tasks(_)
        | ReadOp::Notes(_) => apply(deps, command),
        ReadOp::Status => Asked::Work(Work::Status(None)),
        ReadOp::StatusSection(section) => Asked::Work(Work::Status(Some(section))),
        ReadOp::Schema { refresh } => Asked::Work(Work::Schema { refresh }),
        ReadOp::Explain(sql) => Asked::Work(Work::Explain(sql)),
        // Bare `/sessions` opens the picker a saved session is chosen from, and
        // `list` keeps printing the same rows as words.
        ReadOp::Sessions => Asked::Sessions,
        ReadOp::SessionsList => Asked::Work(Work::SessionsList),
        ReadOp::SessionsDelete(id) => Asked::Work(Work::SessionsDelete(id)),
        ReadOp::Preferences(request) => Asked::Work(Work::Preferences(request)),
        ReadOp::Contracts(request) => Asked::Work(Work::Contracts(request)),
        ReadOp::Recovery(request) => Asked::Work(Work::Recovery(request)),
        ReadOp::Doctor => Asked::Work(Work::Doctor),
        ReadOp::Permissions(request) => Asked::Work(Work::Permissions(request)),
        ReadOp::Sql(sql) => Asked::Work(Work::Sql(sql)),
        // The direct reader holds the surface's last result and claims these
        // three before this table; with none in hand there is nothing to write
        // or to chart, which is exactly what the surface says.
        ReadOp::Export(_) => Asked::Notice(super::direct::NOTHING_TO_EXPORT.to_owned()),
        ReadOp::Chart(_) => Asked::Notice(super::direct::NOTHING_TO_CHART.to_owned()),
        ReadOp::Investigation(request) => Asked::Work(Work::Investigation(request)),
        // The harness run engine is not this surface's (O5): one pointer line
        // to the terminal command that drives it.
        ReadOp::Runs(_) => Asked::Notice(super::direct::RUN_MOVED.to_owned()),
        ReadOp::Help(_) | ReadOp::Connections => apply(deps, command),
    }
}

/// A context change. The conversation's own two — `/clear`, and the one
/// compaction operation `/compact` and `/rotate` state — are the chat layer's
/// rollover, and the session performs them ([`Asked::Roll`]). So is every change
/// of what the session talks to — the model, the provider, the connection, the
/// privacy setting — because the run already open cannot absorb one: its
/// request, its prompt prefix and its provider were composed once, at open
/// (`chat::rollover`). The session-command layer states the new value, says what
/// it stated, and the run is rolled into a successor composed from it, seeded
/// with the conversation so far, before the next message — or, before the first
/// turn, given up and composed again by that message.
pub(super) fn roll(deps: &OpsDeps, command: SlashCommand, request: RolloverRequest) -> Asked {
    match request.change {
        ContextChange::Clear => Asked::Roll {
            said: apply(deps, command).into_words(),
            request,
        },
        ContextChange::Compact | ContextChange::Rotate => Asked::Roll {
            said: String::new(),
            request,
        },
        ContextChange::Connect
        | ContextChange::Include
        | ContextChange::Exclude
        | ContextChange::Provider
        | ContextChange::Model
        | ContextChange::Privacy => Asked::Roll {
            said: apply(deps, command).into_words(),
            request,
        },
        // A saved session's record is opened in place of this one, by the
        // operation both surfaces share (`Asked::Resume`); bare names the
        // records to choose from.
        ContextChange::Resume => Asked::Resume(request.value.unwrap_or_default()),
    }
}

/// Performs `command` on the session's own state, through the one layer that
/// decides what a session command means. An action this table does not word is
/// stated in the shared layer's own terms rather than swallowed.
pub(super) fn apply(deps: &OpsDeps, command: SlashCommand) -> Asked {
    let available = deps
        .runtime
        .current()
        .connections
        .profiles
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut state = deps.state();
    match state.apply(command, &available) {
        SessionAction::Message(words) => Asked::Notice(words),
        SessionAction::Error(words) => Asked::Error(words),
        // `/resume` opens the record it names — the same operation the chat
        // layer's rollover states — and bare opens the picker.
        SessionAction::Resume(id) => Asked::Resume(id),
        // A command whose behaviour is a turn's or a job's never reaches this
        // arm: the chat layer states it as its own operation above.
        other => Asked::Error(format!(
            "this surface cannot perform that session command ({other:?})"
        )),
    }
}

impl Asked {
    /// The words an answer carries, where it carries any.
    fn into_words(self) -> String {
        match self {
            Asked::Notice(words) | Asked::Error(words) => words,
            _ => String::new(),
        }
    }
}

/// The profile list `/connections` names, in today's wording: the one the
/// configuration in force holds, which is what a reload just moved.
pub(super) fn connections_words(runtime: &RuntimeConfig) -> String {
    let names: Vec<&str> = runtime
        .connections
        .profiles
        .keys()
        .map(String::as_str)
        .collect();
    if names.is_empty() {
        return NO_PROFILES.to_owned();
    }
    format!("Profiles: {}", names.join(", "))
}
