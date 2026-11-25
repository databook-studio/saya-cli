//! Rendering one bounded stub.
//!
//! The stub keeps the call (tool and arguments), its outcome and original
//! size, the first and last lines of its body, and either how to recover it or
//! why it is no longer needed. Arguments and excerpt lines are redacted and
//! capped, so a secret in a tool result is never re-sent by this lane.

use super::calls::Call;
use super::{MAX_STUB_BYTES, STUB_SENTINEL};
use serde_json::Value;

const MAX_ARGS_BYTES: usize = 200;
const MAX_EXCERPT_BYTES: usize = 160;
const MAX_TOOL_NAME_BYTES: usize = 80;

/// Why a result is being replaced.
pub(super) enum Kind<'a> {
    /// Old non-read output outside the newest groups.
    Old,
    /// A later call made this result unnecessary.
    Superseded(&'a Call),
    /// A read stubbed only because the history is above its hard ceiling.
    WorkingCopy,
}

/// The bounded replacement for one tool result.
pub(super) fn stub(kind: Kind<'_>, call: &Call, content: &str) -> String {
    let tool = cap(call.name(), MAX_TOOL_NAME_BYTES);
    let raw_arguments = call.arguments().to_string();
    let redacted = saya_types::redact(&raw_arguments);
    let arguments = cap(&redacted, MAX_ARGS_BYTES);
    let header = match &kind {
        Kind::Old => format!("{STUB_SENTINEL} earlier tool result reduced to bound the request]"),
        Kind::Superseded(_) => format!("{STUB_SENTINEL} superseded by a later tool call]"),
        Kind::WorkingCopy => format!("{STUB_SENTINEL} reduced above the hard history ceiling]"),
    };
    let mut stub = format!(
        "{header}\n\
         tool: {tool}\n\
         arguments: {arguments}\n\
         outcome: {} ({} bytes)\n",
        outcome(content),
        content.len(),
    );
    match kind {
        Kind::Old => stub.push_str(&re_read_line(tool)),
        Kind::Superseded(later) => stub.push_str(&format!(
            "superseded by: {} {}\n",
            cap(later.name(), MAX_TOOL_NAME_BYTES),
            cap(later.id(), MAX_TOOL_NAME_BYTES),
        )),
        Kind::WorkingCopy => stub.push_str(
            "note: this is the latest read of the file; re-read the file before editing\n",
        ),
    }
    if let Some(first) = excerpt(content, true) {
        stub.push_str(&format!("first: {first}\n"));
    }
    if let Some(last) = excerpt(content, false) {
        stub.push_str(&format!("last: {last}\n"));
    }
    cap(&stub, MAX_STUB_BYTES).to_string()
}

/// Whether a result carries an error, as the call classifier reads it.
pub(super) fn is_error(content: &str) -> bool {
    matches!(
        serde_json::from_str::<Value>(content),
        Ok(value) if value.get("error").is_some()
    )
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
            format!("re-read: run the same statement again with {tool}\n")
        }
        "unknown tool" => "re-read: the call is unavailable; issue it again\n".to_string(),
        _ => format!("re-read: call {tool} again with the same arguments to see the full result\n"),
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
    &text[..super::super::super::tools::floor_boundary(text, max)]
}
