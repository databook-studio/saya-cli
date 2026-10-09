use crate::ToolDefinition;
use serde_json::Value;

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
