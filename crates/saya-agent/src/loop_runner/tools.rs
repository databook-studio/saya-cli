#[cfg(test)]
use crate::CancellationToken;
use crate::{ToolCall, ToolDefinition, ToolExecutionContext, ToolExecutor};
use saya_types::redact;
#[cfg(test)]
use saya_types::redact_counted;
use serde_json::Value;

pub(super) use super::tool_batch::{complete_tool, execute_batch, turn_result_caps};
pub(super) use super::tool_shape::tool_message;
pub use super::tool_shape::{
    MAX_TOOL_MESSAGE_BYTES, ShapedToolResult, shape_tool_result, tool_message_cap,
};

/// Why a tool call cannot run as requested, or `None` when it is valid.
///
/// Recoverable problems (unknown tool, non-object arguments) are fed back to
/// the model as a tool result so it can correct itself; aborting the whole
/// run over one hallucinated name would waste the entire multi-turn effort.
/// A call with no id cannot anchor a well-formed tool message, so the caller
/// treats that case as fatal regardless of the reason returned here.
pub(super) fn invalid_reason(call: &ToolCall, definitions: &[ToolDefinition]) -> Option<String> {
    if !call.name.is_empty() && definitions.iter().any(|tool| tool.name == call.name) {
        if call.arguments.is_object() {
            return None;
        }
        return Some("arguments must be a JSON object".into());
    }
    let available = definitions
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "unknown tool '{}'; available tools: {available}",
        call.name
    ))
}

/// The completion summaries reported for a tool call, derived from the
/// call's definition: the definition's own `completion` text when it
/// declares one — a tool whose action is neither a read-only database read
/// nor a local-state write (e.g. one that writes a file and opens a browser)
/// states what it actually did — otherwise the generic wording keyed on the
/// declared `read_only`. A write tool (`read_only: false`, e.g. one that
/// persists a candidate claim) must not read as a "read-only" completion —
/// that would be a false statement in the feature whose pitch is that it
/// does not overstate what it knows. The failure summary keeps the word
/// "failed" for a useful model-facing explanation. Turn-tool metadata and
/// failure memory use the typed execution outcome instead of interpreting
/// that wording. `None` (no definition found) falls back to the write
/// wording, matching the pre-definition lookup behavior for a call whose
/// definition is absent.
pub(super) fn completion_summaries(
    definition: Option<&ToolDefinition>,
    name: &str,
    arguments: &Value,
    result: Option<&Value>,
) -> (String, String) {
    let read_only = definition.is_some_and(|definition| definition.read_only);
    let (completed, failed) = if read_only {
        (
            "read-only database tool completed",
            "read-only database tool failed",
        )
    } else {
        ("local-state write completed", "local-state write failed")
    };
    match definition.and_then(|definition| definition.completion.as_deref()) {
        Some(text) => (
            completion_detail(name, text, arguments, result, false),
            completion_detail(name, text, arguments, result, true),
        ),
        None => (completed.to_owned(), failed.to_owned()),
    }
}

/// Shapes one completion summary for the tools this slice covers,
/// leaving every other tool's text byte-exact.
///
/// `workspace_write` names the file from the call's `path` argument
/// (`notes.md written`); `workspace_edit` names the file from `path` plus
/// the edit's own completion (`notes.md edited`); `run_command` names the
/// program from `program` plus the outcome from the tool result's `exit_code`
/// (`pytest exited 1`).
/// The failure arm never carries the success completion text: with a key
/// fact it reads `failed <key fact>` (e.g. `failed notes.md`,
/// `failed pytest`), and without one it reads `failed <name>` — both keep
/// the substring "failed" without reusing the success verb. This also fixes
/// the old contradiction, where a failed host call read as
/// `failed to complete: host command ran` (failed *and* "ran").
///
/// Only the model-supplied arguments and the tool's own typed result feed
/// the key fact — never stdout/stderr or file content — so the detail rides
/// the same redaction the rest of the output does: there is no tool output
/// in the line to redact. A success carries no reason beyond the key fact:
/// the `Err` path reaches `completion_summaries` with `result: None`, so no
/// exit code is reachable there (a non-zero exit is an `Ok` outcome, not a
/// failure), and the `ToolError` text itself is appended to the generic
/// failure summary separately in [`execute`] (bounded, redacted) rather than
/// threaded through these summaries — a bare `failed <key fact>` is the
/// honest form here. Uncovered tools fall
/// back to today's text untouched; a covered tool missing its key still
/// must not reuse the success text, so it reads `failed <name>`.
fn completion_detail(
    name: &str,
    base: &str,
    arguments: &Value,
    result: Option<&Value>,
    failed: bool,
) -> String {
    if failed {
        let key: Option<String> = match name {
            "workspace_write" | "workspace_edit" => arguments
                .get("path")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            "run_command" => arguments
                .get("program")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            _ => None,
        };
        return match (name, key) {
            ("workspace_write" | "workspace_edit" | "run_command", Some(key)) => {
                format!("failed {key}")
            }
            ("workspace_write" | "workspace_edit" | "run_command", None) => {
                format!("failed {name}")
            }
            (_, _) => format!("failed to complete: {base}"),
        };
    }
    let key = match name {
        "workspace_write" => arguments
            .get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|path| format!("{path} written")),
        "workspace_edit" => arguments
            .get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|path| format!("{path} edited")),
        "run_command" => arguments
            .get("program")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|program| {
                let outcome = match result.and_then(|value| value.get("exit_code")) {
                    Some(Value::Number(code)) => code
                        .as_i64()
                        .map(|code| format!("exited {code}"))
                        .unwrap_or_else(|| "ran".to_owned()),
                    _ => "ran".to_owned(),
                };
                format!("{program} {outcome}")
            }),
        _ => None,
    };
    match key {
        Some(key) => key,
        None => base.to_owned(),
    }
}

/// Runs a tool and returns its result with a completion summary that reflects
/// the tool's *declaration*, not its name (see [`completion_summaries`]). The
/// The typed outcome returned beside the summary drives turn-tool status and
/// failure memory; summary text remains model-facing only.
#[cfg(test)]
pub(super) async fn execute(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
    definition: Option<&ToolDefinition>,
) -> (Value, String) {
    execute_with_context(
        tools,
        name,
        arguments,
        definition,
        ToolExecutionContext {
            result_cap: MAX_TOOL_MESSAGE_BYTES,
            cancellation: CancellationToken::new(),
        },
    )
    .await
}

pub(super) async fn execute_with_context(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
    definition: Option<&ToolDefinition>,
    context: ToolExecutionContext,
) -> (Value, String) {
    let (result, summary, _) =
        execute_with_outcome(tools, name, arguments, definition, context).await;
    (result, summary)
}

pub(super) async fn execute_with_outcome(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
    definition: Option<&ToolDefinition>,
    context: ToolExecutionContext,
) -> (Value, String, bool) {
    match tools
        .execute_with_context(name, arguments.clone(), context)
        .await
    {
        Ok(value) => {
            let (completed, _) = completion_summaries(definition, name, &arguments, Some(&value));
            (value, completed, true)
        }
        // The reason reaches the model so it can adjust (e.g. a
        // safety-layer rejection naming what is not allowed); the bounded,
        // redacted form reaches the human summary so a refusal reads
        // differently from a runtime failure.
        Err(error) => {
            let (_, failed) = completion_summaries(definition, name, &arguments, None);
            let summary = failure_summary_with_reason(&failed, &error.to_string());
            (
                serde_json::json!({"error": error.to_string()}),
                summary,
                false,
            )
        }
    }
}
/// Ceiling on how many tool calls from one assistant message run at once.
/// This scheduler bound is independent of connector or profile pool settings:
/// calls may target separate connections, so it is not a claim about one
/// shared database pool.
pub(super) const MAX_CONCURRENT_TOOL_CALLS: usize = 4;

/// Cap on the failure reason carried into the human-facing summary, in
/// characters. Long enough to name a safety rejection (the canonical refusal
/// is well under half this) and short enough that one failure cannot flood
/// the transcript or a persisted session. The bound is the summary only —
/// the model's JSON still carries the full error.
const MAX_FAILURE_REASON_CHARS: usize = 200;

/// Appends a failure reason to the generic failure summary so a safety-layer
/// refusal reads differently from a runtime failure. The reason is the
/// untrusted error text, so it is redacted (secret-shaped material must not
/// reach `TerminalEvent` payloads, per `security.md`), single-lined, and
/// bounded at [`MAX_FAILURE_REASON_CHARS`] with the existing `…` marker.
/// Keeps "failed" in model-facing wording and offers no override — the line
/// explains what was refused, never how to force it. Typed outcomes, rather
/// than this substring, drive turn-tool status and failure memory.
fn failure_summary_with_reason(failed: &str, error: &str) -> String {
    let reason = bound_failure_reason(&redact(error));
    if reason.is_empty() {
        return failed.to_owned();
    }
    format!("{failed} — {reason}")
}

/// Single-lines `reason` and cuts it to [`MAX_FAILURE_REASON_CHARS`]
/// characters, marking the cut with `…`. Operates on characters (not bytes)
/// so the slice never splits a multi-byte sequence, and on an already
/// redacted string — redaction runs first so secret-shaped material cannot
/// hide inside the truncated tail.
fn bound_failure_reason(reason: &str) -> String {
    let single_line = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let single_line = single_line.trim();
    if single_line.is_empty() {
        return String::new();
    }
    let bounded: String = single_line.chars().take(MAX_FAILURE_REASON_CHARS).collect();
    if bounded.len() < single_line.len() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

/// Largest byte index `<= idx` that falls on a UTF-8 character boundary, so a
/// truncation slice never splits a multi-byte sequence. (`str::floor_char_boundary`
/// would do this but is stable only since 1.91, above the 1.88 MSRV.)
pub(super) fn floor_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        idx = text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
