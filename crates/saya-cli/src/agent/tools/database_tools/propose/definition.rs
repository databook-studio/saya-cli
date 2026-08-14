//! The `contract_propose` tool definition.
//!
//! `database_data: true` — a contract is derived from the user's database and
//! is subject to the same sharing gate as the read tools. `requires_approval:
//! false` is deliberate: the *permission* gate is `permit_candidate_writes`
//! (enforced by the loop and by registration), and a candidate is inert until a
//! human confirms it; prompting per proposal would train click-through on the
//! dialogs that guard SQL (spec 3c §1). `local_state: WriteCandidate` is the
//! first such declaration; the loop refuses it unless writes are permitted.
//!
//! Registered only when writes are permitted, a store is present, and the
//! privacy gate is open — see `database_tools::definitions`. When the gate is
//! closed the tool is **hidden**, not advertised-as-always-denied, matching the
//! 2b-3a precedent (SPEC REVIEW, Q1): a tool the model can see but that never
//! works spends context and invites retries.

use saya_agent::{LocalStateEffect, ToolDefinition, ToolEffect};
use serde_json::json;

/// The `kind` values the tool advertises, in declaration order. One source of
/// truth: the schema's `enum` is built from this, and the argument validator
/// checks the model's `kind` against the same list.
pub(super) const KIND_ENUM: &[&str] = &[
    "description",
    "alias",
    "grain",
    "time-column",
    "column-description",
    "column-role",
];

/// The `contract_propose` tool definition. `read_only: false` — this is the
/// first agent tool that writes local state (a candidate claim).
pub(crate) fn definition() -> ToolDefinition {
    // Built from `KIND_ENUM` so the schema's `enum` and the argument validator
    // share one source of truth for the kind vocabulary.
    let kind_enum = serde_json::to_value(KIND_ENUM).expect("kind enum serializes to a JSON array");
    ToolDefinition {
        name: "contract_propose".into(),
        description: "Propose one candidate contract claim about a database object, derived from \
            what you observed this turn. The claim is stored as a candidate — inert until a human \
            confirms it; you cannot confirm a claim yourself. Use this when the user's question or \
            your queries reveal a durable fact about a table or column (an alias, a grain, a \
            default time column, a description, or a column role). Pass `table` as exactly three \
            dot-separated parts: catalog.schema.object. `kind` is one of: description, alias, \
            grain, time-column, column-description (needs `column`), column-role (needs \
            `column`, value is the role: identifier, dimension, measure, timestamp, sensitive). \
            At most eight proposals are stored per turn; the ninth is refused."
            .into(),
        read_only: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "table": {
                    "type": "string",
                    "description": "Fully qualified object name: catalog.schema.object."
                },
                "kind": {
                    "type": "string",
                    "enum": kind_enum,
                    "description": "The kind of contract claim to propose."
                },
                "value": { "type": "string", "description": "The claim value." },
                "column": {
                    "type": "string",
                    "description": "Required for column-description and column-role; the column."
                },
                "connection": {
                    "type": "string",
                    "description": "Optional. Name of the database connection (profile) whose \
                        local contract memory to write to; defaults to the primary."
                }
            },
            "required": ["table", "kind", "value"],
            "additionalProperties": false
        }),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            // See the module docs: the permission gate is permit_candidate_writes.
            requires_approval: false,
            local_state: LocalStateEffect::WriteCandidate,
        },
    }
}
