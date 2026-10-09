use saya_agent::{LocalStateEffect, ToolDefinition, ToolEffect};

use super::{MAX_ARG_BYTES, MAX_ARG_COUNT, RUN_PROGRAM_TOOL, RunProgram};

impl RunProgram {
    /// The tool definition.
    pub fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: RUN_PROGRAM_TOOL.to_string(),
            description: "Run one allowlisted program with typed argv. Every argument is \
                passed verbatim as one argv element — no shell, no interpolation, no \
                command-line string anywhere. The allowlist is this step's narrowed set; \
                bash, sh, wrappers, and paths are refused. The child runs sandboxed in \
                this run's workspace; output is capped and redacted; a timeout kills the \
                whole process group."
                .into(),
            read_only: false,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "program": { "type": "string" },
                    "args": {
                        "type": "array",
                        "maxItems": MAX_ARG_COUNT,
                        "items": { "type": "string", "maxLength": MAX_ARG_BYTES }
                    },
                    "timeout_seconds": { "type": "integer", "minimum": 1 }
                },
                "required": ["program"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: !self.spawn.net_allow().is_empty(),
                // Plan-gated, like scratch: approving the plan approves the
                // programs it names. The child writes inside the run
                // workspace's fs roots.
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: saya_agent::ToolConcurrency::Serial,
            completion: Some("program ran".into()),
        }
    }
}
