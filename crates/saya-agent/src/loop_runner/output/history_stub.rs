//! Per-request history policy: old tool results become bounded stubs.
//!
//! A provider request does not need to re-send an old file read, command
//! output or search result in full on every later turn. The model already saw
//! it, and those bytes dominate the prompt. Before each request, every
//! tool-result group older than the last [`WHOLE_TOOL_TURNS`] is rewritten in
//! place to a bounded stub that keeps the call (tool and arguments), its
//! outcome and original size, the first and last lines, and how to re-read the
//! full result.
//!
//! The rewrite is one-way and idempotent: once stubbed, a message is
//! byte-stable across every later request, so the request prefix the provider
//! caches keeps hitting through the stubbed region and only the newest whole
//! turns change. Arguments and excerpt lines pass through
//! [`saya_types::redact`], and the whole stub is capped, so a secret in a tool
//! result is never re-sent by this lane.
//!
//! This is not the budget trim: [`super::trim_to_budget`] never rewrites an
//! already-sent message, while this policy deliberately replaces old results
//! because the stub preserves how to recover them. The latest
//! [`WHOLE_TOOL_TURNS`] groups stay whole, so the turn the model is reasoning
//! over is never reduced.

use crate::{ChatMessage, ToolCall};
use serde_json::Value;

/// How many trailing tool-result groups stay whole. The newest group is the
/// turn just executed; one more gives the model the immediately preceding
/// result to compare against. Older groups become stubs.
pub(in crate::loop_runner) const WHOLE_TOOL_TURNS: usize = 2;

/// Leading sentinel on a stubbed tool result, so a later request recognises it
/// and leaves it alone.
pub(super) const STUB_SENTINEL: &str = "…[history-stub:";

/// Hard cap on one stub. A stub is a pointer back to the result, not a
/// summary of it; keeping it small is the point of the policy.
pub(in crate::loop_runner) const MAX_STUB_BYTES: usize = 900;

/// Per-field caps that keep one stub under [`MAX_STUB_BYTES`] without a final
/// cut through the re-read line.
const MAX_ARGS_BYTES: usize = 200;
const MAX_EXCERPT_BYTES: usize = 160;
const MAX_TOOL_NAME_BYTES: usize = 80;

/// Replaces every tool result in a group older than the last
/// [`WHOLE_TOOL_TURNS`] with a bounded stub, in place, and returns how many
/// messages it rewrote. Idempotent: an already-stubbed message is left as is.
pub(in crate::loop_runner) fn stub_old_tool_results(
    messages: &mut [ChatMessage],
    protected_prefix_len: usize,
) -> usize {
    let groups = super::group_trim::tool_groups(messages, protected_prefix_len);
    let Some(older) = groups.len().checked_sub(WHOLE_TOOL_TURNS) else {
        return 0;
    };
    let mut stubbed = 0;
    for range in &groups[..older] {
        // The call arguments live on the group's assistant message; copy the
        // calls before mutating the results so a tool message can find its own.
        let calls = messages[range.start].tool_calls.clone();
        for message in &mut messages[range.start + 1..range.end] {
            if message.role != "tool" || message.content.starts_with(STUB_SENTINEL) {
                continue;
            }
            let call = message
                .tool_call_id
                .as_deref()
                .and_then(|id| calls.iter().find(|call| call.id == id));
            message.content = stub(call, &message.content);
            stubbed += 1;
        }
    }
    stubbed
}

/// The bounded replacement for one old tool result.
fn stub(call: Option<&ToolCall>, content: &str) -> String {
    let tool = cap(
        call.map_or("unknown tool", |call| call.name.as_str()),
        MAX_TOOL_NAME_BYTES,
    );
    let arguments = call.map_or_else(
        || "(unavailable)".to_string(),
        |call| {
            let raw = serde_json::to_string(&call.arguments).unwrap_or_default();
            cap(&saya_types::redact(&raw), MAX_ARGS_BYTES).to_string()
        },
    );
    let mut stub = format!(
        "{STUB_SENTINEL} earlier tool result reduced to bound the request]\n\
         tool: {tool}\n\
         arguments: {arguments}\n\
         outcome: {} ({} bytes)\n\
         {}\n",
        outcome(content),
        content.len(),
        re_read_line(tool),
    );
    if let Some(first) = excerpt(content, true) {
        stub.push_str(&format!("first: {first}\n"));
    }
    if let Some(last) = excerpt(content, false) {
        stub.push_str(&format!("last: {last}\n"));
    }
    cap(&stub, MAX_STUB_BYTES).to_string()
}

/// The outcome class of a result: an error, a cut result, or a normal one.
fn outcome(content: &str) -> &'static str {
    match serde_json::from_str::<Value>(content) {
        Ok(value) if value.get("error").is_some() => "error",
        Ok(value) if value.get("truncated").and_then(Value::as_bool) == Some(true) => "truncated",
        Ok(_) => "ok",
        Err(_) if content.contains("...[truncated]") => "truncated",
        Err(_) => "unparsed",
    }
}

/// How the model gets the full result back. A database read is re-run as the
/// same statement; any other tool is called again with the same arguments,
/// subject to its own approval and safety rules.
fn re_read_line(tool: &str) -> String {
    match tool {
        "bounded_sql_query" | "bounded_sql_query_all" => {
            format!("re-read: run the same statement again with {tool}")
        }
        "unknown tool" => "re-read: the call is unavailable; issue it again".to_string(),
        _ => format!("re-read: call {tool} again with the same arguments to see the full result"),
    }
}

/// The first or last non-empty line of the result's text body, redacted and
/// capped. A JSON result's envelope is not the body, so a known text field
/// (`content`, `stdout`, a search match) is used when present.
fn excerpt(content: &str, first: bool) -> Option<String> {
    let text = text_body(content).unwrap_or_else(|| content.to_string());
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let line = if first {
        lines.next()
    } else {
        lines.next_back()
    }?;
    let redacted = saya_types::redact(line);
    Some(cap(&redacted, MAX_EXCERPT_BYTES).to_string())
}

/// The human-readable body of a JSON tool result, when it has one.
fn text_body(content: &str) -> Option<String> {
    let value: Value = serde_json::from_str(content).ok()?;
    for key in ["content", "stdout", "output", "text", "body", "stderr"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    let matches = value.get("matches").and_then(Value::as_array)?;
    let first = matches.first()?.get("text").and_then(Value::as_str)?;
    let last = matches.last()?.get("text").and_then(Value::as_str)?;
    Some(format!("{first}\n{last}"))
}

/// `text` truncated to at most `max` bytes on a character boundary.
fn cap(text: &str, max: usize) -> &str {
    &text[..super::super::tools::floor_boundary(text, max)]
}
