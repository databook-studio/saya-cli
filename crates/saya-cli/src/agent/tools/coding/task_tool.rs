//! The direct-mode `helpers` tool: the root hands independent pieces to helper
//! children. Every helper works in the same working checkout as the root, so
//! there is no per-helper checkout and no merge step; the call waits for its
//! helpers and returns a redacted summary per helper. Advertised only by
//! `DirectCodingTools`; the orchestration lives in `commands::coding::task_flow`.

use crate::commands::coding::{CodingError, NativeCodingJob};
use saya_agent::ToolDefinition;
use serde_json::{Value, json};
use std::sync::Arc;

/// The `helpers` tool definition. One call is one helper, so `dependencies`
/// never applies, and several calls in one turn run at the same time (the tool
/// is `Concurrent`), each assignment capped at 32 KiB
/// (`MAX_HELPER_ASSIGNMENT_BYTES`). Every helper works in the root's own working
/// checkout, so there are no per-helper paths to declare. Advertised only by
/// `DirectCodingTools`.
///
/// The schema advertises exactly the types the job's registry holds, and the
/// description states each one's shape from that same registry, so what the
/// model is offered and what it is told cannot disagree. A job with no
/// `[agents]` advertises the two built-ins.
pub(crate) fn definition(agents: &saya_config::HelperAgents) -> ToolDefinition {
    let mut definition = super::definition(
        "helpers",
        &format!(
            "{}Launch one helper: one call is one helper, and several calls in one turn run \
             at the same time. Every helper works in the same checkout as you, so give each \
             call a separate piece of the work and keep their edits apart. The helper has not \
             seen this conversation, so its prompt must stand alone: say what to do, which \
             paths, and whether to edit or only report — do not restate the specification. \
             {} {}",
            super::OPTIONAL_DELEGATION,
            agent_types(agents),
            super::REQUEST_AUTHORITY_RULE
        ),
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "description": "A few words naming this helper."
                },
                "prompt": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32768,
                    "description": "Self-contained assignment. The helper has not seen this conversation. Scope and paths only; do not restate the specification."
                },
                "subagent_type": {
                    "type": "string",
                    "enum": agents.iter().map(|(name, _)| name).collect::<Vec<_>>(),
                    "description": agent_summary(agents),
                },
                "agent_id": {
                    "type": "string",
                    "description": "Optional. Omit it and one is assigned."
                }
            },
            "required": ["description", "prompt", "subagent_type"],
            "additionalProperties": false
        }),
        true,
    );
    // Launching a helper is not itself a workspace write — each helper's own
    // writes and shell calls go through its own approvals — and the coding root
    // auto-approves its advertised `helpers` definition, so a per-call approval
    // buys nothing. It does cost the overlap: the runner only batches calls it
    // can allow without an approval, so requiring one would run several
    // `helpers` calls in one turn serially. The `local_state` stays
    // `WriteWorkspace` (the tool is write-shaped: it launches writers), so a job
    // that does not permit workspace writes still refuses it at the workspace
    // gate, exactly as before.
    definition.effect.requires_approval = false;
    definition
}

/// The advertised types with their shapes, in registry order, as the tool
/// description states them: one sentence per type, in full.
fn agent_types(agents: &saya_config::HelperAgents) -> String {
    agents
        .iter()
        .map(|(name, agent)| {
            if agent.allow_write {
                format!("subagent_type `{name}` implements one piece.")
            } else {
                format!(
                    "subagent_type `{name}` reads only: it can read and search, and it never edits."
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The same types, short, for the `subagent_type` parameter's own description:
/// the parameter already carries the full sentence above it.
fn agent_summary(agents: &saya_config::HelperAgents) -> String {
    agents
        .iter()
        .map(|(name, agent)| {
            let shape = if agent.allow_write {
                "implements one piece"
            } else {
                "reads only"
            };
            format!("`{name}` {shape}.")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs the `helpers` tool: delegate, wait until the helper children finish, and
/// return a redacted summary per helper. Direct mode only; `task_run` enforces
/// the mode guard.
pub(crate) async fn execute(job: &Arc<NativeCodingJob>, args: Value) -> Result<Value, CodingError> {
    job.task_run(args).await
}

#[cfg(test)]
#[path = "task_tool_tests.rs"]
mod task_tool_tests;
