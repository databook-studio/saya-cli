//! The one canonical vocabulary of model-tool names.
//!
//! Two lists, and every tool name any surface can advertise is in exactly one
//! of them:
//!
//! - [`GOVERNED_TOOLS`] — the tools `[tools] allow` / `--tools` may name. They
//!   are the local, capability-shaped tools: what the agent may touch in the
//!   workspace, on the machine, or over the network.
//! - [`DATABASE_TOOLS`] — the tools that are *not* governed, because their
//!   availability is already decided by something the operator states
//!   elsewhere: selecting a connection or a profile, and turning data sharing
//!   on. Naming one in the list is a typed error
//!   ([`super::ToolsAllowError::DatabaseTool`]) rather than a silent no-op, so
//!   a user cannot believe they narrowed a surface they did not.
//!
//! The drift test in `saya-cli` fails when a surface advertises a name that is
//! in neither list, so a new tool cannot silently bypass the allow list.

/// The tool names `[tools] allow` may name, canonical and de-duplicated by
/// construction. `bash` is not here: it is an alias of `shell`
/// ([`canonical_tool_name`]), and an allow list is stored canonically.
pub const GOVERNED_TOOLS: &[&str] = &[
    "checklist",
    "delete",
    "edit",
    "glob",
    "grep",
    "helpers",
    "http_download",
    "http_fetch",
    "list",
    "notebook_edit",
    "read",
    "request_clarification",
    "rewind",
    "run_command",
    "shell",
    "shell_kill",
    "shell_output",
    "submit_plan",
    "web_fetch",
    "web_search",
    "write",
];

/// The tool names `[tools] allow` must not name. Every one of them is tied to
/// selecting a connection or a profile and, except for `schema_discovery`, to
/// the data-sharing setting — the bounded SQL family, the query fan-out, the
/// contract tools that read a state store — and none of them is a capability
/// the operator grants by naming it.
pub const DATABASE_TOOLS: &[&str] = &[
    "bounded_sql_query",
    "bounded_sql_query_all",
    "column_health",
    "contract_read",
    "contract_search",
    "designate_answer",
    "join_check",
    "result_shape",
    "schema_discovery",
];

/// The namespace prefix every MCP-sourced tool name carries. The double
/// underscore is the separator, so a server name may never contain one
/// ([`crate::is_valid_server_name`]) and the server a tool name belongs to is
/// the segment between the first and second separator.
pub const MCP_TOOL_PREFIX: &str = "mcp__";

/// A tool name's own character ceiling, after its `mcp__<server>__` prefix.
/// Long enough for the names real servers use, short enough that the whole
/// advertisement stays a bounded part of the request.
pub const MAX_MCP_TOOL_NAME_CHARS: usize = 64;

/// The prefix an MCP tool name carries for the server it belongs to, or
/// `None` for a name that is not an MCP name at all.
///
/// A name is an MCP name when it is `mcp__<server>` — the server's whole
/// entry, which an allow list may state — or `mcp__<server>__<tool>`, one tool
/// of it. The server segment is judged by the same rule the config table uses
/// ([`crate::is_valid_server_name`]), so a name no configured server could
/// produce is not a name an allow list can state.
pub fn mcp_server_of(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(MCP_TOOL_PREFIX)?;
    let (server, tool) = match rest.split_once("__") {
        Some((server, tool)) => (server, Some(tool)),
        None => (rest, None),
    };
    if !crate::is_valid_server_name(server) {
        return None;
    }
    if let Some(tool) = tool
        && !is_valid_mcp_tool_segment(tool)
    {
        return None;
    }
    name.get(..MCP_TOOL_PREFIX.len() + server.len())
}

/// Whether `name` names one tool of an MCP server — the
/// `mcp__<server>__<tool>` form — rather than the server's whole entry.
pub fn is_mcp_tool_call_name(name: &str) -> bool {
    mcp_server_of(name).is_some_and(|prefix| prefix.len() < name.len())
}

/// One tool segment: non-empty, bounded, and drawn from the characters a
/// provider's function-name field accepts. A server that names a tool
/// anything else has that tool dropped, never renamed into a name another
/// tool already answers to.
pub fn is_valid_mcp_tool_segment(tool: &str) -> bool {
    !tool.is_empty()
        && tool.chars().count() <= MAX_MCP_TOOL_NAME_CHARS
        && tool
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Whether `name` is a name `[tools] allow` may state for an MCP server's
/// tools: the server's whole entry (`mcp__<server>`) or one exact tool of it
/// (`mcp__<server>__<tool>`).
pub fn is_mcp_allow_name(name: &str) -> bool {
    mcp_server_of(name).is_some()
}

/// The canonical spelling of a model-tool name. `bash` is the model's spelling
/// of `shell`; every other name is taken as written. One table, so an allow
/// list, an advertisement, and a dispatch cannot disagree about which tool a
/// word names.
pub fn canonical_tool_name(name: &str) -> &str {
    match name {
        "bash" => "shell",
        other => other,
    }
}

/// The name an allow list states and matches a tool by. The shell's two
/// readers are parts of the one `shell` capability — a start with no reader
/// would be a promise nothing could keep — so a list that names `shell` keeps
/// the whole family, and a list that names a reader states `shell`.
pub fn allow_list_name(name: &str) -> &str {
    match canonical_tool_name(name) {
        "shell_output" | "shell_kill" => "shell",
        other => other,
    }
}

/// Whether `name` — any spelling — is one of the tools `[tools] allow` may
/// name.
///
/// The MCP family is governed by shape rather than by a list: the names come
/// from third-party servers the configuration names, so no fixed catalog can
/// hold them. Every `mcp__<server>__<tool>` a configured server advertises is
/// therefore a name the list may take away, and [`super::ToolsAllow::parse`]
/// refuses a malformed one rather than admitting a name no server could have
/// produced.
pub fn is_governed_tool(name: &str) -> bool {
    let canonical = canonical_tool_name(name);
    GOVERNED_TOOLS.contains(&canonical) || is_mcp_allow_name(canonical)
}

/// Whether `name` — any spelling — is one of the connection-tied tools the
/// allow list must not name.
pub fn is_database_tool(name: &str) -> bool {
    DATABASE_TOOLS.contains(&canonical_tool_name(name))
}
