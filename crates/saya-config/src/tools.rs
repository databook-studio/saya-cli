//! The `[tools]` setting: one allow list that narrows every agent surface.
//!
//! One setting governs the interactive session, `saya ask`, `saya run`, and
//! coding jobs, root and helper alike (AGENTS.md, "One operation, multiple
//! adapters"). Each surface assembles its tools exactly as it did before and
//! then keeps only the names this list admits, so the list can only ever take
//! a tool away: a name never grants a tool whose own condition fails —
//! `shell` under `--approval-mode never` stays hidden however the list is
//! spelled.
//!
//! [`catalog`] holds the two name lists and the alias table; [`allow`] holds
//! the validated set and its typed errors; [`resolve`] turns the file's raw
//! words into that set. Nothing here reads a database: the connection-tied
//! tools are refused by name ([`ToolsAllowError::DatabaseTool`]) because their
//! availability follows the selected connection, not this list.

mod allow;
mod catalog;
mod merge;
mod resolve;

pub use allow::{ToolsAllow, ToolsAllowError, database_tool_names, governed_tool_names};
pub use catalog::{
    DATABASE_TOOLS, GOVERNED_TOOLS, MAX_MCP_TOOL_NAME_CHARS, MCP_TOOL_PREFIX, allow_list_name,
    canonical_tool_name, is_database_tool, is_governed_tool, is_mcp_allow_name,
    is_mcp_tool_call_name, is_valid_mcp_tool_segment, mcp_server_of,
};
pub use resolve::ResolvedTools;

pub(crate) use merge::merge;
pub(crate) use resolve::resolve;

use serde::Deserialize;

/// Raw trusted `[tools]` settings. Every field stays optional until layers
/// merge, so a higher-precedence layer can override one setting at a time.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolsFile {
    /// The tools the surfaces may advertise. `None` — the default — narrows
    /// nothing. The whole table is on the protected list, so an untrusted
    /// project layer cannot state it.
    pub allow: Option<Vec<String>>,
}
