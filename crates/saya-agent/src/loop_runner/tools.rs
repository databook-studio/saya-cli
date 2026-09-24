use crate::{AgentLimits, ChatMessage, LocalStateEffect, ToolCall, ToolDefinition, ToolExecutor};
use saya_types::{redact, redact_counted};
use serde_json::Value;

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

/// Whether the runner must refuse to run `definition` unattended because it
/// has an external side effect. A tool that touches the world outside the
/// agent must go through approval; when it already requires approval this
/// gate is satisfied by the prompt, so the term only denies a tool that set
/// `external_side_effect` without also setting `requires_approval` — a
/// misconfiguration the loop refuses rather than trusting every author to
/// set both. The one exception is a run constructed with
/// `AgentLimits::permit_external_effects`: the plan-gated egress permit for
/// tools whose effect the run's approved scope already approved once with
/// its plan, not per call. Everywhere the permit is false the guard's
/// purpose stands unchanged, which is what keeps interactive turns
/// byte-identical. Both execution paths consult this, so a tool the
/// policy gates cannot be auto-run by one path and not the other.
pub(super) fn external_side_effect_gated(
    definition: &ToolDefinition,
    limits: &AgentLimits,
) -> bool {
    definition.effect.external_side_effect
        && !definition.effect.requires_approval
        && !limits.permit_external_effects
}

/// Whether the runner must refuse `definition` because it may write a
/// candidate claim and the run was not constructed with candidate writes
/// permitted (Phase 3a: fail closed by default).
pub(super) fn candidate_denied(definition: &ToolDefinition, limits: &AgentLimits) -> bool {
    definition.effect.local_state == LocalStateEffect::WriteCandidate
        && !limits.permit_candidate_writes
}

/// Whether the runner must refuse `definition` because it may write files in
/// the run workspace and the run was not constructed with workspace writes
/// permitted — fail closed by default, mirroring [`candidate_denied`].
pub(super) fn workspace_write_denied(definition: &ToolDefinition, limits: &AgentLimits) -> bool {
    definition.effect.local_state == LocalStateEffect::WriteWorkspace
        && !limits.permit_workspace_writes
}

/// May a call to `definition` run with no questions asked — the single
/// policy the loop consults to decide auto-run. A call is auto-runnable only
/// when it needs no approval, the policy does not gate its external side
/// effect, and it is not a local-state write the runner refused. The batch
/// path calls this to decide whether the calls in a message are independent
/// enough to run concurrently. The sequential execution path resolves
/// approval and then re-applies the gates by name in `run_turn_tools`
/// (currently [`external_side_effect_gated`] and [`candidate_denied`]) so the
/// denial can name which one refused — a gate added here must be mirrored
/// there, or it binds multi-call turns only.
pub(super) fn auto_runnable(definition: &ToolDefinition, limits: &AgentLimits) -> bool {
    !definition.effect.requires_approval
        && !external_side_effect_gated(definition, limits)
        && !candidate_denied(definition, limits)
        && !workspace_write_denied(definition, limits)
}

/// The completion summaries reported for a tool call, derived from the
/// call's definition: the definition's own `completion` text when it
/// declares one — a tool whose action is neither a read-only database read
/// nor a local-state write (e.g. one that writes a file and opens a browser)
/// states what it actually did — otherwise the generic wording keyed on the
/// declared `read_only`. A write tool (`read_only: false`, e.g. one that
/// persists a candidate claim) must not read as a "read-only" completion —
/// that would be a false statement in the feature whose pitch is that it
/// does not overstate what it knows. The failure summary always keeps the
/// substring "failed": `tool_metadata.status` and the call-outcome
/// memory derive their failure signal from it. `None` (no definition found)
/// falls back to the write wording, matching the pre-definition lookup
/// behavior for a call whose definition is absent.
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
/// summary drives `tool_metadata.status` via a `contains("failed")` check, so
/// every failure string keeps the substring "failed".
pub(super) async fn execute(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
    definition: Option<&ToolDefinition>,
) -> (Value, String) {
    match tools.execute(name, arguments.clone()).await {
        Ok(value) => {
            let (completed, _) = completion_summaries(definition, name, &arguments, Some(&value));
            (value, completed)
        }
        // The reason reaches the model so it can adjust (e.g. a
        // safety-layer rejection naming what is not allowed); the bounded,
        // redacted form reaches the human summary so a refusal reads
        // differently from a runtime failure.
        Err(error) => {
            let (_, failed) = completion_summaries(definition, name, &arguments, None);
            let summary = failure_summary_with_reason(&failed, &error.to_string());
            (serde_json::json!({"error": error.to_string()}), summary)
        }
    }
}
/// Executes already-validated, auto-runnable calls concurrently while
/// preserving input order in the returned results.
///
/// The tool-call list comes from the model, so its size is untrusted:
/// `max_tool_calls` is a whole-run *total*, not a simultaneity cap, so a
/// message emitting twenty calls would otherwise fan out twenty concurrent
/// database queries. Concurrency is bounded here by
/// [`MAX_CONCURRENT_TOOL_CALLS`]: `buffered` caps how many futures are polled
/// at once and still yields results in input order. Actual database fan-out is
/// bounded again by the connection pool, which defaults to four connections
/// (see `saya-connectors`' factory), so a cap above the pool size only queues
/// inside the connector — `MAX_CONCURRENT_TOOL_CALLS` matches that default.
pub(super) async fn execute_batch(
    tools: &dyn ToolExecutor,
    calls: &[ToolCall],
    definitions: &[ToolDefinition],
) -> Vec<(Value, String)> {
    use futures_util::{StreamExt, stream};
    let pending = calls.iter().map(|call| {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == call.name);
        let name = call.name.clone();
        let arguments = call.arguments.clone();
        async move { execute(tools, &name, arguments, definition).await }
    });
    stream::iter(pending)
        .buffered(MAX_CONCURRENT_TOOL_CALLS)
        .collect()
        .await
}

/// Ceiling on how many tool calls from a single assistant message run at the
/// same instant. The connection pool defaults to four connections
/// (`saya-connectors`' factory), so fanning out more than this only queues
/// inside the pool — matching the pool default keeps the cap meaningful without
/// over-subscribing the database.
const MAX_CONCURRENT_TOOL_CALLS: usize = 4;

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
/// Keeps the "failed" substring the status derivation and failure memory key
/// on, and offers no override — the line explains what was refused, never
/// how to force it.
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

/// Builds the `tool`-role message for a result, truncating it to fit the
/// conversation byte budget when a single result would otherwise exceed it,
/// and scrubbing secret-shaped material at the model-context boundary (D8).
/// Returns the message plus whether truncation was applied so the caller can
/// mark the completion summary — the model must not silently believe it saw a
/// complete result.
///
/// `byte_budget` is the loop's whole-conversation bound
/// (`AgentLimits::context_byte_budget`); a single tool message is capped below
/// it so one result can never, by itself, breach the budget and abort the run
///. `MAX_TOOL_MESSAGE_BYTES` is a separate, provider-facing
/// hard ceiling kept well under any provider's per-message limit.
///
/// The scrub (M2-3b) is the D8 boundary rule: *every* tool result entering
/// model context — SQL rows included — passes `redact()` after the byte cap,
/// before the message reaches the provider, so secret-shaped material
/// (`key=value`, credential headers, userinfo URLs, PEM blocks) cannot cross
/// to the model. Ordinary values pass through unchanged. The scrub governs the
/// model's context only: the event stream and `saya query` keep the raw bytes
/// (the user's own data on their own machine), so an answer may quote
/// `[redacted]` where the raw value once appeared. Redacting after the cap can
/// grow the content past `cap` by at most the marker length per match; the
/// cap is a budget bound, not a provider hard limit.
///
/// D8 follow-up: an ordinary `token=next_token` or `password=args.password`
/// shape is unremarkable in source code, and a model that reads its own file
/// back and sees `[redacted]` cannot tell a masked secret from corruption —
/// observed calling it "a corruption" and rewriting the file; writing the
/// shown text back would replace the real value on disk. When
/// [`redact_counted`] changed anything, the content is prefixed
/// once with a fixed note naming the count, so the model has the signal it
/// needs to leave the masked span alone rather than "fix" it. A clean result
/// gets no note and is byte-identical to before this change — the redaction
/// itself is unchanged, only the announcement is new.
pub(super) fn tool_message(id: String, result: Value, byte_budget: usize) -> (ChatMessage, bool) {
    let cap = tool_message_cap(byte_budget);
    let (content, truncated) = bounded_json(&result, cap);
    let (content, redacted_count) = redact_counted(&content);
    let content = if redacted_count > 0 {
        format!("{}\n\n{content}", redaction_note(redacted_count))
    } else {
        content
    };
    (
        ChatMessage {
            role: "tool".into(),
            content,
            tool_calls: Vec::new(),
            tool_call_id: Some(id),
        },
        truncated,
    )
}

/// The fixed note prepended to a tool result [`redact_counted`] changed:
/// names the count, restates that the source on disk is unchanged, and tells
/// the model what not to do. Wording is fixed and count-parameterized only,
/// so the model sees the identical shape on every call and can learn it.
fn redaction_note(count: usize) -> String {
    format!(
        "[saya: {count} secret-shaped value(s) in this result were replaced with \
         [redacted]; the source is unchanged — do not write [redacted] back]"
    )
}

/// The per-tool-message cap the loop truncates at, derived from the loop's
/// whole-conversation byte budget: `min(byte_budget, MAX_TOOL_MESSAGE_BYTES)`.
/// Exported beside [`MAX_TOOL_MESSAGE_BYTES`] so a producer of large tool
/// results (the fetch lane's rendered blocks) can pre-bind its own output to
/// the exact number the loop will enforce — the declared bound and the
/// truncation point are the same number by construction and cannot drift.
pub fn tool_message_cap(byte_budget: usize) -> usize {
    byte_budget.min(MAX_TOOL_MESSAGE_BYTES)
}

/// Absolute per-tool-message ceiling, independent of the conversation budget:
/// no provider is asked to ingest a tool result larger than this. Kept below
/// the 16 MiB a connector can return (`saya-connectors`' `MAX_RESULT_BYTES`)
/// so the loop's own bounds, not the connector's, govern what reaches the model.
pub const MAX_TOOL_MESSAGE_BYTES: usize = 65_536;

fn bounded_json(value: &Value, cap: usize) -> (String, bool) {
    let text = serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"error\":\"tool result unavailable\"}".into());
    if text.len() <= cap {
        (text, false)
    } else {
        // Truncate the serialized result to `cap` and append a visible marker
        // so the model knows the data was cut, preserving the leading bytes it
        // can still reason about instead of discarding the whole result.
        let marker = "…[truncated: tool result exceeded the conversation byte budget]";
        let head = cap.saturating_sub(marker.len());
        let mut truncated = String::from(&text[..floor_boundary(&text, head)]);
        truncated.push_str(marker);
        (truncated, true)
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
