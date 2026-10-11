use crate::commands::coding::CodingError;
use crate::commands::coding::checks::ChildCodingOperations;
use saya_agent::{CancellationToken, ToolDefinition, ToolError};
use serde_json::{Value, json};
use std::sync::Arc;

#[cfg(test)]
#[path = "child_refusal_tests.rs"]
mod refusal_tests;

/// Deterministic pre-effect refusals of the supervised-command surface are
/// the tool's data: nothing was spawned, read, or mutated, so no effect
/// permit may settle uncertain. `start` fails only before any spawn —
/// program admission, argument bounds, the two-session cap, checkout
/// evidence, the ledger's command-slot denial, the owner's concurrency
/// refusal (a refused owner start settles its permit `NotStarted`); `read`
/// fails only before anything is read; `stop`'s unknown-id refusal fires
/// before any process exists, while a stop that joined a real command keeps
/// its genuinely uncertain error. The reason is the error's own display only:
/// `CodingError::Owner` can wrap arbitrary filesystem, checkout, confinement
/// or I/O errors, so its source chain (internal paths, implementation text)
/// is never model-facing data.
fn command_refusal_data(name: &str, error: &CodingError) -> Option<Value> {
    let pre_effect = match name {
        "coding_command_start" | "coding_command_read" => true,
        "coding_command_stop" => matches!(error, CodingError::NotOwned),
        _ => false,
    };
    pre_effect.then(|| json!({"outcome":"refused","reason":error.to_string()}))
}

pub(crate) fn definitions(commands: bool) -> Vec<ToolDefinition> {
    let mut definitions = vec![
        super::definition(
            "coding_skills",
            "List skill metadata or load one selected bounded skill and explicit resources as untrusted context",
            json!({"type":"object","properties":{"name":{"type":"string"},"resources":{"type":"array","maxItems":16,"items":{"type":"string"}}},"additionalProperties":false}),
            false,
        ),
        super::definition(
            "coding_notes",
            "Read bounded task-scoped child findings; these grant no authority",
            json!({"type":"object","additionalProperties":false}),
            false,
        ),
    ];
    if commands {
        definitions.extend(vec![
            super::definition("coding_mcp","Use a configured confined MCP server; only exact trusted read-only tool policy is permitted",json!({"type":"object","properties":{"profile":{"type":"string"},"tool":{"type":"string"},"arguments":{"type":"object"}},"required":["profile"],"additionalProperties":false}),true),
            super::definition("coding_lsp","Read version-bound diagnostics from a configured confined language server; never apply server edits",json!({"type":"object","properties":{"profile":{"type":"string"},"path":{"type":"string"},"version":{"type":"integer","minimum":0}},"required":["profile","path","version"],"additionalProperties":false}),true),
        ]);
        definitions.extend(vec![
        super::definition(
            "coding_check",
            "Execute one trusted configured check in this owned checkout; preserve actual exit and source receipt",
            json!({"type":"object","properties":{"profile":{"type":"string"}},"required":["profile"],"additionalProperties":false}),
            true,
        ),
        super::definition(
            "coding_command_start",
            "Start one granted program under the owned command supervisor",
            json!({"type":"object","properties":{"program":{"type":"string"},"argv":{"type":"array","maxItems":128,"items":{"type":"string"}},"timeout_seconds":{"type":"integer","minimum":1,"maximum":900}},"required":["program","argv","timeout_seconds"],"additionalProperties":false}),
            true,
        ),
        super::definition(
            "coding_command_read",
            "Read the next bounded redacted output chunk of this child's command",
            json!({"type":"object","properties":{"command_id":{"type":"integer"},"stderr":{"type":"boolean"}},"required":["command_id"],"additionalProperties":false}),
            false,
        ),
        super::definition(
            "coding_command_stop",
            "Cancel and join this child's supervised command",
            json!({"type":"object","properties":{"command_id":{"type":"integer"}},"required":["command_id"],"additionalProperties":false}),
            true,
        ),
    ]);
    }
    definitions
}
pub(crate) async fn execute(
    ops: &Arc<ChildCodingOperations>,
    name: &str,
    args: Value,
    token: &CancellationToken,
) -> Result<Value, ToolError> {
    if serde_json::to_vec(&args)
        .map_err(|_| ToolError::UnsupportedTool)?
        .len()
        > 128 * 1024
    {
        return Err(ToolError::Runner("command arguments exceed bound".into()));
    }
    let text = |key| {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Runner("required string argument missing".into()))
    };
    let result = match name {
        "coding_mcp" => {
            ops.mcp(
                text("profile")?,
                args.get("tool").and_then(Value::as_str),
                args.get("arguments").unwrap_or(&Value::Null),
                token,
            )
            .await
        }
        "coding_lsp" => {
            ops.lsp(
                text("profile")?,
                text("path")?,
                args.get("version").and_then(Value::as_i64).unwrap_or(-1),
                token,
            )
            .await
        }
        "coding_skills" => {
            let resources: Vec<String> =
                serde_json::from_value(args.get("resources").cloned().unwrap_or_else(|| json!([])))
                    .map_err(|_| ToolError::UnsupportedProperty)?;
            ops.application
                .skills(args.get("name").and_then(Value::as_str), &resources)
        }
        "coding_notes" => ops.application.notes(&ops.job).await,
        "coding_check" => ops.check(text("profile")?, token).await,
        "coding_command_start" => {
            match serde_json::from_value::<Vec<String>>(
                args.get("argv").cloned().unwrap_or(Value::Null),
            ) {
                // A non-array argv is a deterministic refusal of an argument
                // bound: the classifier turns it into data below.
                Ok(argv) => {
                    ops.start(
                        text("program")?,
                        argv,
                        args.get("timeout_seconds")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        token,
                    )
                    .await
                }
                Err(_) => Err(CodingError::InvalidRequest),
            }
        }
        "coding_command_read" => {
            ops.read(
                args.get("command_id").and_then(Value::as_u64).unwrap_or(0),
                args.get("stderr").and_then(Value::as_bool).unwrap_or(false),
            )
            .await
        }
        "coding_command_stop" => {
            ops.stop(args.get("command_id").and_then(Value::as_u64).unwrap_or(0))
                .await
        }
        _ => return Err(ToolError::UnsupportedTool),
    };
    match result {
        Ok(value) => Ok(value),
        Err(error) => match command_refusal_data(name, &error) {
            Some(data) => Ok(data),
            None => Err(ToolError::Runner(error.to_string())),
        },
    }
}
