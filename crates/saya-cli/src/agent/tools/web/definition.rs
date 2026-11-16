//! The `web_fetch` schema: one definition, byte-identical on every surface
//! that advertises it, and the name table that routes it.

use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use serde_json::json;

use super::bounds::{DEFAULT_BODY_BYTES, MAX_URL_BYTES, MIN_BODY_BYTES, max_body_bytes};

/// The tool's name. `web_fetch` is what the model calls; there is no alias,
/// because the name is not a shell and nothing else spells it differently.
pub(crate) const NAME: &str = "web_fetch";

/// Whether a model's tool name is this tool — the one name table a dispatch
/// and an advertisement both read, so they cannot disagree.
pub(crate) fn is_web_fetch(name: &str) -> bool {
    name == NAME
}

/// The `web_fetch` definition. Its effect is the `shell`'s shape for the same
/// reason: the call reaches the world outside the workspace, so it carries
/// `external_side_effect` and `requires_approval`, and it writes nothing here.
/// `read_only` is true because the call changes nothing of the agent's — the
/// two flags answer different questions, and the model reads this one to know
/// whether a call can be repeated safely.
pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: NAME.into(),
        description: "Fetches one https URL and returns the page as text. An HTML page is \
            converted to readable text; any other content type is passed through unchanged. A \
            page longer than max_bytes is cut, with the elided byte count marked. The user is \
            asked before each fetch."
            .into(),
        read_only: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "maxLength": MAX_URL_BYTES,
                    "description": "The https URL to fetch."
                },
                "max_bytes": {
                    "type": "integer",
                    "minimum": MIN_BODY_BYTES,
                    "maximum": max_body_bytes(),
                    "default": DEFAULT_BODY_BYTES,
                    "description": "Byte bound on the returned text. A longer page is cut \
                        head and tail, with the elided byte count marked in the text."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: Some("web page fetched".into()),
    }
}
