use crate::cli::{InvestigationCommand, PreferencesCommand};
use crate::commands::recovery::RecoveryCommand;
use crate::interactive::session::lifecycle::notes::NotesCommand;
use saya_agent::PermissionMode;
use std::fmt;

pub(crate) mod catalog;
mod contracts;
mod help;
mod investigation;
mod job;
pub(crate) mod param_redact;
pub(crate) mod registry;
pub(crate) mod user_commands;

// The contract slash request (the one operation plus the note a deprecated
// spelling states), re-exported so `SlashCommand::Contracts` and
// `SessionAction::Contracts` name the same type from every layer.
pub(crate) use contracts::ContractsRequest;
// Re-exported so the session command layer's `crate::slash::help_for` path
// resolves through the `help` module.
pub(crate) use help::help_for;
// The bypass-composition sentence is shared by the `/mode` help entry and the
// `/mode` answers, so it is re-exported for the session command layer.
pub(crate) use help::PLAN_BYPASS_SENTENCE;
// The run scope parser's equivalence tests read the `/run` help from
// `commands/run/scopes.rs`, so the per-command help joins the re-exports.
#[cfg(test)]
pub(crate) use help::command_help;
// The inline `test_help_command` test calls `help_text` bare via `super::*`;
// bring it into scope for tests only so the test stays unchanged.
#[cfg(test)]
use help::help_text;

include!("slash/command_types.rs");
include!("slash/request_types.rs");
include!("slash/export_parser.rs");
include!("slash/report_parser.rs");
include!("slash/preferences_parser.rs");
include!("slash/notes_parser.rs");
include!("slash/recovery_parser.rs");
include!("slash/rewind_parser.rs");
include!("slash/dispatch.rs");
include!("slash/value_parsers.rs");
#[cfg(test)]
#[path = "slash/tests.rs"]
mod tests;
