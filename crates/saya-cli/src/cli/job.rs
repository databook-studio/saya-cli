//! The `saya job` command's rule: which of the shared verbs the command runs, and
//! what it answers the rest.
//!
//! `saya job` shares the one `CodingCommand` grammar rather than restating its
//! twenty lifecycle verbs: a second declaration would be a second grammar to
//! keep in step, and clap has no way to say "these twenty of those
//! twenty-eight". The command's rule lives here instead — the agent-shaped
//! controls are hidden from help and refused by name, naming the spelling
//! that does work: the two launches have a CLI spelling of their own, and the
//! child controls are `/job`'s verbs in a session (`ref/03-commands.md` row
//! 171), so they name their `/job` verb.
//! `commands/coding/lane.rs` applies it.

use super::CodingCommand;

/// The spelling that runs this command, or `None` when it is one of the
/// command's own twenty lifecycle verbs. The slash parser reads its own map, so
/// the CLI command and the session name one spelling each — and a child control
/// is refused here naming the `/job` verb the session runs it as.
pub(crate) fn session_spelling(command: &CodingCommand) -> Option<&'static str> {
    Some(match command {
        CodingCommand::Start { .. } => "saya start <id> <task> …",
        CodingCommand::FollowUp { .. } => "saya follow-up <id> <agent> …",
        CodingCommand::Approve { .. } => "/job approve <id>",
        CodingCommand::Deny { .. } => "/job deny <id>",
        CodingCommand::Message { .. } => "/job message <id> <agent> <text>",
        CodingCommand::Pause { .. } => "/job pause <id> <agent>",
        CodingCommand::Continue { .. } => "/job continue <id> <agent>",
        CodingCommand::CancelChild { .. } => "/job cancel-child <id> <agent>",
        _ => return None,
    })
}

/// The refusal `saya job` answers an agent-shaped control with: one sentence,
/// naming the spelling that does work.
pub(crate) fn refusal(command: &CodingCommand) -> Option<String> {
    session_spelling(command).map(|spelling| {
        format!(
            "`saya job` runs the durable-job verbs; this one has its own spelling: `{spelling}`"
        )
    })
}
