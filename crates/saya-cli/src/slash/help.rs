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
///
/// An alias answers with the command it spells — `/help approvals` is
/// `/help mode` — so a deprecated spelling keeps its help for the release it
/// still works in.
pub(crate) fn command_help(name: &str) -> Option<&'static str> {
    let clean_name = name.trim_start_matches('/').to_lowercase();
    let own = |name: &str| {
        command_help_connection_provider(name)
            .or_else(|| command_help_query(name))
            .or_else(|| command_help_session(name))
            .or_else(|| command_help_jobs(name))
            .or_else(|| command_help_memory_runs(name))
    };
    // A spelling with help of its own keeps it — `/contract` says it is an
    // alias in its own sentence — and anything else that the catalogue folds
    // answers with the command it spells: `/help approvals` is `/help mode`.
    own(&clean_name).or_else(|| {
        crate::slash::catalog::resolve(&clean_name)
            .and_then(|canonical| (canonical != clean_name).then(|| own(canonical)))
            .flatten()
    })
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
