//! The `web_search` schema: one definition, byte-identical on every surface
//! that advertises it, and the name table that routes it.

use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use serde_json::json;

use super::bounds::{
    DEFAULT_MAX_RESULTS, MAX_DOMAIN_BYTES, MAX_DOMAINS, MAX_QUERY_BYTES, MAX_RESULTS,
};

/// The tool's name. `web_search` is what the model calls; there is no alias,
/// because the name is not a shell and nothing else spells it differently.
pub(crate) const NAME: &str = "web_search";

/// Whether a model's tool name is this tool — the one name table a dispatch
/// and an advertisement both read, so they cannot disagree.
pub(crate) fn is_web_search(name: &str) -> bool {
    name == NAME
}

/// The `web_search` definition. Its effect is `web_fetch`'s shape for the
/// same reason: the call reaches the world outside the workspace, so it
/// carries `external_side_effect` and `requires_approval`, and it writes
/// nothing here. `read_only` is true because the call changes nothing of the
/// agent's — the two flags answer different questions, and the model reads
/// this one to know whether a call can be repeated safely.
///
/// The description says what the tool does and nothing about *which* backend
/// answers: that is the user's configuration, and the model has no decision
/// to make about it.
pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: NAME.into(),
        description: "Searches the web and returns up to max_results results, each with its \
            title, URL, and a snippet. allowed_domains keeps only results on those domains; \
            blocked_domains drops them. The user is asked before each search."
            .into(),
        read_only: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "maxLength": MAX_QUERY_BYTES,
                    "description": "The search query."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_RESULTS,
                    "default": DEFAULT_MAX_RESULTS,
                    "description": "How many results to return."
                },
                "allowed_domains": {
                    "type": "array",
                    "maxItems": MAX_DOMAINS,
                    "items": { "type": "string", "maxLength": MAX_DOMAIN_BYTES },
                    "description": "Keep only results on these domains and their subdomains."
                },
                "blocked_domains": {
                    "type": "array",
                    "maxItems": MAX_DOMAINS,
                    "items": { "type": "string", "maxLength": MAX_DOMAIN_BYTES },
                    "description": "Drop results on these domains and their subdomains."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: Some("web search complete".into()),
    }
}
