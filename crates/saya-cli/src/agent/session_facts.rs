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

use crate::connection::ConnectionRegistry;

/// Session-stable facts: what the session is, not what it may do.
///
/// Computed once per session from the connection set and the bound workspace
/// root, then passed into every turn's prompt assembly unchanged — the same
/// inputs produce byte-identical text, so the system block keeps one
/// prefix-cache key across turns.
pub(crate) struct SessionFacts<'a> {
    /// The turn's connection registry (the session's connection set).
    pub(crate) registry: &'a ConnectionRegistry,
    /// The session's bound workspace root, when one binds.
    pub(crate) workspace_root: Option<&'a Path>,
}

/// Heading for the session-facts section.
pub(crate) const SESSION_FACTS_HEADING: &str = "Session facts";

/// Renders the session-facts section, or `None` when the session binds
/// neither a connection nor a workspace root — no empty heading.
///
/// Names the connections in scope (name plus engine) and the bound root.
/// States no tool and no permission: the adjacent multi-connection paragraph
/// already carries the per-connection navigation instruction, and the tool
/// schemas are the authority on what is possible.
pub(crate) fn session_facts_text(facts: &SessionFacts<'_>) -> Option<String> {
    let mut lines = vec![SESSION_FACTS_HEADING.to_string()];
    let mut has_facts = false;
    for (name, entry) in facts.registry.entries() {
        lines.push(format!("- Database `{name}` ({}).", entry.dialect.as_str()));
        has_facts = true;
    }
    match facts.workspace_root {
        Some(root) => {
            lines.push(format!("- Workspace root: {}.", root.display()));
            has_facts = true;
        }
        None => {
            if !facts.registry.is_empty() {
                lines.push("- No workspace is bound.".to_string());
                has_facts = true;
            }
        }
    }
    if facts.registry.is_empty() && facts.workspace_root.is_some() {
        lines.push("- No database is connected.".to_string());
    }
    if has_facts {
        Some(lines.join("\n"))
    } else {
        None
    }
}
