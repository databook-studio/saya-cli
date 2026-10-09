use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use saya_types::Capabilities;

use crate::scratch::validate::MAX_SQL_BYTES;

use super::{SCRATCH_IMPORT_TOOL, SCRATCH_SQL_TOOL, ScratchSql};

impl ScratchSql {
    /// The tool definition. The write is plan-gated, not per-call approved:
    /// the scope is approved once at plan approval, so `requires_approval`
    /// stays false and admission above is what refuses the tool. The declared
    /// effect is the honest write-shaped local state — the run's own scratch
    /// file — which read-only approval denies and the loop refuses unless the
    /// runner was constructed with the write permission on.
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: SCRATCH_SQL_TOOL.to_string(),
            description: "Run one statement against this run's scratch database — the run's \
                only writable SQL. It holds the run's staged intermediate results: CREATE \
                TABLE, INSERT, UPDATE, DELETE, and SELECT over them, joins and scoring \
                included. Single statement per call; results are capped at 50 rows. No file \
                reads of any kind — read_csv, read_parquet, ATTACH, COPY, INSTALL and LOAD \
                are refused — so stage corpus data through the workspace tools first."
                .into(),
            read_only: false,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "sql": { "type": "string", "maxLength": MAX_SQL_BYTES }
                },
                "required": ["sql"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                // Scratch is run-scoped state, never a user-registered database.
                database_data: false,
                // No egress: external access is off and locked at open.
                external_side_effect: false,
                // Plan-gated, not per-call approved (ADR 0003).
                requires_approval: false,
                // The write-shaped local state the effect machinery already
                // knows how to gate.
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: ToolConcurrency::Serial,
            completion: Some("scratch SQL executed".into()),
        }
    }

    /// The importer is a scratch-state write, but it is available only when
    /// the composition also bound a workspace.
    pub fn import_definition() -> ToolDefinition {
        ToolDefinition {
            name: SCRATCH_IMPORT_TOOL.to_string(),
            description: "Load one CSV file from this run's bound workspace into one scratch \
                table. The path is relative to that workspace; absolute paths, escapes, \
                symlinks, non-regular files, and files over 32 MiB are refused. Every column \
                becomes VARCHAR. CSV parsing is bounded; scratch itself keeps external access \
                off, so this is the only file-to-scratch route."
                .into(),
            read_only: false,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "table": { "type": "string" },
                    "header": { "type": "boolean", "default": true },
                    "delimiter": { "type": "string", "minLength": 1, "maxLength": 1, "default": "," },
                    "if_exists": { "type": "string", "enum": ["fail", "replace"], "default": "fail" }
                },
                "required": ["path", "table"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: ToolConcurrency::Serial,
            completion: Some("workspace CSV imported into scratch".into()),
        }
    }

    /// The tool's definitions when — and only when — the scratch scope is
    /// approved; empty (hidden, not advertised as always-empty) otherwise.
    pub fn definitions(capabilities: &Capabilities) -> Vec<ToolDefinition> {
        if capabilities.scratch {
            vec![Self::definition()]
        } else {
            Vec::new()
        }
    }
}
