//! Session-stable facts for the system prompt: what the session *is*.
//!
//! States the connections in scope and whether a workspace root is bound —
//! never what the model may do. Capability truth lives in the tool schemas;
//! prose restating the tool list drifts false, so this section names no tool.
//!
//! Session-stable by construction: the inputs are the connection set and the
//! bound root, both fixed for the session (computed once per session, passed
//! in — never read per turn). Approval mode is deliberately absent: it can
//! change mid-session via `/approvals`, which would churn the cache key.

use std::path::Path;

/// Heading for the session-facts section.
pub(crate) const SESSION_FACTS_HEADING: &str = "Session facts";

/// The workspace-root line, in the section's own wording: the one copy both
/// the whole section and a run that states only this fact render.
pub(crate) fn workspace_root_line(root: &Path) -> String {
    format!("- Workspace root: {}.", root.display())
}

/// The same line under the section's heading, for a run whose connection lines
/// are already stated — the database plugin's section states them, so repeating
/// them here would state one fact twice.
pub(crate) fn workspace_root_text(root: &Path) -> String {
    format!("{SESSION_FACTS_HEADING}\n{}", workspace_root_line(root))
}

#[cfg(test)]
#[path = "session_facts_tests.rs"]
mod tests;
