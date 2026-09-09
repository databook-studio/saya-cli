use crate::{AgentLimits, ChatMessage, LocalStateEffect, ToolCall, ToolDefinition, ToolExecutor};
use saya_types::redact;
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
/// set both. Both execution paths consult this, so a tool the
/// policy gates cannot be auto-run by one path and not the other.
pub(super) fn external_side_effect_gated(definition: &ToolDefinition) -> bool {
    definition.effect.external_side_effect && !definition.effect.requires_approval
}

/// Whether the runner must refuse `definition` because it may write a
/// candidate claim and the run was not constructed with candidate writes
/// permitted (Phase 3a: fail closed by default).
pub(super) fn candidate_denied(definition: &ToolDefinition, limits: &AgentLimits) -> bool {
    definition.effect.local_state == LocalStateEffect::WriteCandidate
        && !limits.permit_candidate_writes
}

/// May a call to `definition` run with no questions asked — the single
/// policy the loop consults to decide auto-run. A call is auto-runnable only
/// when it needs no approval, the policy does not gate its external side
/// effect, and it is not a candidate write the runner refused. The batch path
/// calls this to decide whether the calls in a message are independent enough
/// to run concurrently; the sequential execution path applies the same gates
/// (via [`external_side_effect_gated`] and [`candidate_denied`]) after
/// resolving approval, so adding a gate here cannot apply to one path and not
/// the other.
pub(super) fn auto_runnable(definition: &ToolDefinition, limits: &AgentLimits) -> bool {
    !definition.effect.requires_approval
        && !external_side_effect_gated(definition)
        && !candidate_denied(definition, limits)
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
/// substring "failed": `tool_metadata.status` and the statement-outcome
/// memory derive their failure signal from it. `None` (no definition found)
/// falls back to the write wording, matching the pre-definition lookup
/// behavior for a call whose definition is absent.
pub(super) fn completion_summaries(definition: Option<&ToolDefinition>) -> (String, String) {
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
        Some(text) => (text.to_owned(), format!("failed to complete: {text}")),
        None => (completed.to_owned(), failed.to_owned()),
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
    let (completed, failed) = completion_summaries(definition);
    match tools.execute(name, arguments).await {
        Ok(value) => (value, completed),
        // The reason reaches the model so it can adjust (e.g. a
        // safety-layer rejection naming what is not allowed).
        Err(error) => (serde_json::json!({"error": error.to_string()}), failed),
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
pub(super) fn tool_message(id: String, result: Value, byte_budget: usize) -> (ChatMessage, bool) {
    let cap = byte_budget.min(MAX_TOOL_MESSAGE_BYTES);
    let (content, truncated) = bounded_json(&result, cap);
    let content = redact(&content);
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

/// Absolute per-tool-message ceiling, independent of the conversation budget:
/// no provider is asked to ingest a tool result larger than this. Kept below
/// the 16 MiB a connector can return (`saya-connectors`' `MAX_RESULT_BYTES`)
/// so the loop's own bounds, not the connector's, govern what reaches the model.
const MAX_TOOL_MESSAGE_BYTES: usize = 65_536;

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
