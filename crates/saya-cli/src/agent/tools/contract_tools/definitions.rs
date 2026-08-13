//! Tool definitions for the read-only contract tools.
//!
//! Two tools, both `database_data: true` (a contract is derived from a user's
//! database and is subject to the same sharing gate) and `requires_approval:
//! false` (a local read that mutates nothing — the SQL approval flow exists to
//! gate *execution*, and asking the user to approve a memory lookup would train
//! them to click through prompts). See
//! .claude/specs/spec-2b3a-agent-contract-tools.md §1.
//!
//! The `terms` array is capped at 16: an unbounded term list is untrusted input
//! driving a lexical scan over every stored object. Argument validation lives
//! in [`super::validation`].

use saya_agent::{ToolDefinition, ToolEffect};

/// Maximum number of search terms a single `contract_search` call accepts.
pub(super) const MAX_TERMS: usize = 16;

/// Returns the contract tool definitions. Both are advertised only when a state
/// store is present **and** database context is allowed for the active provider
/// (the caller passes both flags); when the privacy gate forbids database
/// context, the tools are hidden rather than advertised as always-empty — a tool
/// the model can see but that never works wastes context and invites retries.
pub(crate) fn definitions(allow_query_data: bool, has_state_store: bool) -> Vec<ToolDefinition> {
    if !allow_query_data || !has_state_store {
        return Vec::new();
    }
    let connection_prop = serde_json::json!({
        "type": "string",
        "description": "Optional. Name of the database connection (profile) whose local contract \
            memory to read; defaults to the primary. Available connections are listed in the \
            system context."
    });
    vec![
        ToolDefinition {
            name: "contract_search".into(),
            description: "Search the local contract memory (confirmed claims the user remembered \
                about their database objects) for objects matching the given terms. Returns \
                bounded, confirmed summaries — candidate claims are not included. A contract is \
                derived from the user's database, so this tool is unavailable when database \
                context sharing is disabled. Pass up to 16 search terms; matches on alias, the \
                qualified name, or description text are returned, most relevant first."
                .into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "connection": connection_prop.clone(),
                    "terms": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Search terms (aliases, object names, or description text)."
                    }
                },
                "required": ["terms"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: false,
            },
        },
        ToolDefinition {
            name: "contract_read".into(),
            description: "Read one object's exact contract from local memory by its fully \
                qualified `catalog.schema.table` name. Returns that object's confirmed claims \
                (each with its id, kind, origin, status, and value), schema state, and any \
                conflicts. A contract is derived from the user's database, so this tool is \
                unavailable when database context sharing is disabled. The `table` argument \
                must be exactly three dot-separated parts: catalog.schema.object."
                .into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "connection": connection_prop,
                    "table": {
                        "type": "string",
                        "description": "Fully qualified object name: catalog.schema.object."
                    }
                },
                "required": ["table"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: false,
            },
        },
    ]
}
