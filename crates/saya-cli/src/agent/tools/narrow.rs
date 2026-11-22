//! The one place a surface applies `[tools] allow` / `--tools`.
//!
//! Every agent surface composes its tools exactly as it always did and then
//! calls [`narrow`] on the result. Because the list is applied *after*
//! composition it can only take a tool away: a name in the list never grants a
//! tool whose own condition failed, so `shell` under `--approval-mode never`
//! stays hidden however the list is spelled.
//!
//! The connection-tied tools are deliberately absent from the catalog, so a
//! list can never remove `schema_discovery` or the bounded SQL family: their
//! availability follows the selected connection or profile, and `saya-config`
//! refuses a list that names one.
//!
//! A tool the list removes is removed from the *advertisement*, and the loop
//! refuses a call to a name it did not advertise ("unknown tool 'x'; available
//! tools: …"). The composites that dispatch without the loop in front of them
//! — a run step, a coding root, a helper — ask [`admits`] at their own door
//! too, so advertisement and dispatch cannot drift.

use saya_agent::ToolDefinition;
use saya_config::ResolvedTools;

/// Keeps only the definitions `tools` admits, in place. An unstated list
/// (`allow: None`) keeps every definition: the default surface is the one the
/// composition produced, unchanged.
pub(crate) fn narrow(definitions: &mut Vec<ToolDefinition>, tools: &ResolvedTools) {
    definitions.retain(|definition| admits(tools, &definition.name));
}

/// Whether this surface may dispatch `name` at all. The same predicate the
/// advertisement was filtered by, so a call that was never advertised is
/// refused at the door rather than run.
///
/// The connection-tied tools are *not* governed: their availability follows
/// the selected connection or profile, so a list can neither add nor remove
/// one, and `saya-config` refuses a list that names one by name. Everything
/// else is the list's to remove.
pub(crate) fn admits(tools: &ResolvedTools, name: &str) -> bool {
    !saya_config::is_governed_tool(name) || tools.admits(name)
}

#[cfg(test)]
#[path = "narrow_tests.rs"]
mod tests;
