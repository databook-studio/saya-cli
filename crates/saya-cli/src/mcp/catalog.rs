//! The MCP tool catalog: the advertised definitions (task Db). One function
//! per tool; the whole toolset is fixed at startup, and the row-returning
//! tools exist only when data sharing is allowed (invariant 2) — the MCP
//! client is the model host, so rows leaving this process is what the gate
//! governs.

use std::sync::Arc;

use rmcp::model::{Tool, ToolAnnotations, object};
use serde_json::json;

use super::policy::ServePolicy;

/// The tools `tools/list` advertises — everything a client can ever call,
/// built in alphabetical order. Row-returning tools are absent entirely when
/// data sharing is off, not merely marked.
pub(crate) fn advertised(policy: &ServePolicy) -> Vec<Tool> {
    let mut tools = vec![contracts_tool()];
    if policy.allow_data_sharing() {
        tools.push(investigation_run_tool());
    }
    tools.push(list_profiles_tool());
    if policy.allow_data_sharing() {
        tools.push(query_tool());
    }
    tools.push(schema_tool());
    tools
}

/// The two tools that return rows. Listed with data sharing on; refused with
/// it off.
pub(crate) fn is_row_returning(name: &str) -> bool {
    matches!(name, "query" | "investigation_run")
}

fn read_only_tool(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    schema: serde_json::Value,
) -> Tool {
    Tool::new(name, description, Arc::new(object(schema)))
        .with_title(title)
        .with_annotations(ToolAnnotations::new().read_only(true))
}

fn list_profiles_tool() -> Tool {
    read_only_tool(
        "list_profiles",
        "List profiles",
        "List the connection profiles this server may use, with their SQL \
         dialects; never paths, hosts, or identities.",
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        }),
    )
}

fn schema_tool() -> Tool {
    read_only_tool(
        "schema",
        "Schema",
        "Discover the schema of an allowed profile: one compact entry per \
         table with its columns, primary key, and foreign keys.",
        json!({
            "type": "object",
            "properties": {"profile": {"type": "string"}},
            "required": ["profile"],
            "additionalProperties": false,
        }),
    )
}

fn query_tool() -> Tool {
    read_only_tool(
        "query",
        "Query",
        "Run one bounded, read-only SQL statement against an allowed profile \
         and get its rows; the same safety gate the command line uses.",
        json!({
            "type": "object",
            "properties": {
                "profile": {"type": "string"},
                "sql": {"type": "string"}
            },
            "required": ["profile", "sql"],
            "additionalProperties": false,
        }),
    )
}

fn contracts_tool() -> Tool {
    read_only_tool(
        "contracts",
        "Contracts",
        "Look up the confirmed business context (active claims) recorded for \
         an allowed profile: every object's contract, or one object's when \
         `table` (catalog.schema.object) is given.",
        json!({
            "type": "object",
            "properties": {
                "profile": {"type": "string"},
                "table": {"type": "string"}
            },
            "required": ["profile"],
            "additionalProperties": false,
        }),
    )
}

fn investigation_run_tool() -> Tool {
    read_only_tool(
        "investigation_run",
        "Run investigation",
        "Replay one saved investigation against its mapped connection and get \
         its result and evidence; a stale review is refused and is never \
         revalidated from here.",
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "profile": {"type": "string"}
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
    )
}
