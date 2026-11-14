//! The one place an MCP tool's exposed name is built and read back.
//!
//! Every tool a configured server advertises reaches the model as
//! `mcp__<server>__<tool>`, and the model's call comes back with that same
//! name. The construction and the parse live together so an advertisement and
//! a dispatch cannot disagree about which server a name belongs to.
//!
//! The shape is [`saya_config`]'s: the same `mcp_server_of` an allow list is
//! validated against decides here whether a name is an MCP name at all, so a
//! name the config could not govern is not a name this client serves.

use saya_config::{MCP_TOOL_PREFIX, is_valid_mcp_tool_segment, mcp_server_of};

/// The name one of `server`'s tools is exposed under.
///
/// The server name has already been validated by the config table (it holds no
/// `__`), and the tool name by [`is_valid_mcp_tool_segment`], so the result
/// reads back unambiguously.
pub(crate) fn exposed(server: &str, tool: &str) -> String {
    format!("{MCP_TOOL_PREFIX}{server}__{tool}")
}

/// The server and tool an exposed name names, or `None` for a name that is not
/// one of this client's tool names.
pub(crate) fn split(name: &str) -> Option<(&str, &str)> {
    let prefix = mcp_server_of(name)?;
    let server = prefix.strip_prefix(MCP_TOOL_PREFIX)?;
    let tool = name.get(prefix.len() + 2..)?;
    (!tool.is_empty()).then_some((server, tool))
}

/// Whether a server's own tool name may be exposed. A name outside the shape
/// has its tool dropped rather than rewritten: renaming it could collide with
/// another tool of the same server, and a collision would make one of the two
/// unreachable.
pub(crate) fn is_exposable(tool: &str) -> bool {
    is_valid_mcp_tool_segment(tool)
}
