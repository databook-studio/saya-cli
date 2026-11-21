//! Help text for the slash commands: the grouped listing (`/help` with no
//! argument) and the per-command usage + example (`/help <command>`).
//!
//! Extracted from `slash.rs` to keep that file under the size cap. `slash.rs`
//! re-exports [`help_for`] so the existing `crate::slash::help_for` path used by
//! the session command layer keeps resolving.
//!
//! The listing and the one-line descriptions are a view of
//! [`crate::slash::catalog::CATALOGUE`] — the usage form, the description, the
//! heading and the order all come from that one table. The completion popup in
//! `interactive::tui::complete` reads the same rows, so the popup and the
//! listing cannot drift into two hand-maintained copies.

use super::user_commands::{UserCommands, user_command_help, user_commands_section};
use crate::slash::catalog::Category;

include!("help/descriptions.rs");
/// Returns a short usage and example string for a known slash command, or `None` if unknown.
pub(crate) fn command_help(name: &str) -> Option<&'static str> {
    let clean_name = name.trim_start_matches('/').to_lowercase();
    command_help_connection_provider(&clean_name)
        .or_else(|| command_help_query(&clean_name))
        .or_else(|| command_help_session(&clean_name))
        .or_else(|| command_help_jobs(&clean_name))
        .or_else(|| command_help_memory_runs(&clean_name))
}
include!("help/connection_provider.rs");
include!("help/query.rs");
include!("help/session.rs");
include!("help/jobs.rs");
include!("help/memory_runs.rs");
include!("help/help_for.rs");
#[cfg(test)]
#[path = "help/tests.rs"]
mod tests;
