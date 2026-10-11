use crate::cli::{ContractsCommand, InvestigationCommand, PreferencesCommand};
use crate::commands::recovery::RecoveryCommand;
use crate::interactive::session::lifecycle::notes::NotesCommand;
use saya_agent::{AgentMode, ApprovalPolicy};
use std::{fmt, str::FromStr};

mod coding;
mod contracts;
mod help;
mod investigation;
pub(crate) mod param_redact;
pub(crate) mod registry;

// Re-exported so the session command layer's `crate::slash::help_for` path
// still resolves after the help text moved to `help.rs`.
pub(crate) use help::help_for;
// The bypass-composition sentence is shared by the `/mode` help entry and the
// `/mode` answers, so it is re-exported for the session command layer.
pub(crate) use help::PLAN_BYPASS_SENTENCE;
// the one-line description per command is the single source shared by the
// `/help` listing and the completion popup (`interactive::tui::complete`), so
// the two surfaces cannot drift. `description_for` is read by the popup in
// production; `COMMAND_DESCRIPTIONS` is only needed by tests that assert the
// shared table covers the registry, so it is re-exported under `cfg(test)`.
#[cfg(test)]
pub(crate) use help::COMMAND_DESCRIPTIONS;
pub(crate) use help::description_for;
// The run scope parser's parity test reads the `/run` help from
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
include!("slash/dispatch.rs");
include!("slash/value_parsers.rs");
#[cfg(test)]
#[path = "slash/tests.rs"]
mod tests;
