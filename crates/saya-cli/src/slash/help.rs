//! Help text for the slash commands: the grouped listing (`/help` with no
//! argument) and the per-command usage + example (`/help <command>`).
//!
//! Extracted from `slash.rs` to keep that file under the size cap. `slash.rs`
//! re-exports [`help_for`] so the existing `crate::slash::help_for` path used by
//! the session command layer keeps resolving.
//!
//! [`COMMAND_DESCRIPTIONS`] is the single source of the one-line description per
//! command. The completion popup in `interactive::tui::complete` reads it via
//! [`description_for`], and the `/help` listing here reads it too — so the popup
//! and the listing can never drift into two hand-maintained copies. The listing
//! only adds presentation the popup does not carry: a usage form, a group
//! heading, and which of the alias-folded commands to show.

include!("help/registry.rs");
include!("help/descriptions.rs");
include!("help/listing_groups.rs");
/// Returns a short usage and example string for a known slash command, or `None` if unknown.
pub(crate) fn command_help(name: &str) -> Option<&'static str> {
    let clean_name = name.trim_start_matches('/').to_lowercase();
    command_help_connection_provider(&clean_name)
        .or_else(|| command_help_query(&clean_name))
        .or_else(|| command_help_session(&clean_name))
        .or_else(|| command_help_memory_runs(&clean_name))
}
include!("help/connection_provider.rs");
include!("help/query.rs");
include!("help/session.rs");
include!("help/memory_runs.rs");
include!("help/help_for.rs");
#[cfg(test)]
#[path = "help/tests.rs"]
mod tests;
