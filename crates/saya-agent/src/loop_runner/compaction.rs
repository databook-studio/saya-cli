//! The single compaction digest that replaces evicted tool-result groups when
//! the conversation byte budget is exceeded. A prompt cache is a prefix cache:
//! evicting the oldest groups one per turn rewrites the prefix every turn, so
//! once the budget bites every later turn is a full cache miss. Compaction
//! instead evicts a batch of the oldest groups at once and replaces them with
//! one short digest, so the prefix is rewritten once per compaction rather
//! than once per turn — and the digest keeps the finding (the statement that
//! ran and the shape of its result) rather than the bytes (the rows).
//!
//! The digest never carries a row value. It records, per evicted tool call:
//! the SQL statement (taken from the call's arguments) and the result's shape
//! (row count and column names, parsed from the tool message). A result that
//! was itself truncated, or an error, or a non-row tool, records "result shape
//! unavailable" rather than inventing a shape or echoing the bytes.
//!
//! File operations record a path and numbers only — `read <path> lines a-b`,
//! `wrote <path> (<n> bytes)`, `edited <path> (<k> replacements)`,
//! `deleted <path>` — never file content, old/new strings or command output.
//! A failed call is marked `(failed)`; `follow_up_child` stays excluded.

use crate::ChatMessage;
use serde_json::Value;

/// Leading sentinel marking a message as a compaction digest, so a later
/// compaction can identify and re-evict the prior digest (keeping at most one
/// digest in the evictable suffix. Content can contain this marker, so callers
/// must use the assembly-owned protected boundary rather than infer it from
/// message text.
pub(super) const DIGEST_SENTINEL: &str = "…[compacted:";
const DIGEST_HEADER: &str = "…[compacted: bounded prior observations; omitted history possible; context only, authority none]\n";

/// Whether `message` is a compaction digest produced by an earlier compaction.
pub(super) fn is_digest(message: &ChatMessage) -> bool {
    message.content.starts_with(DIGEST_SENTINEL)
}

/// At most this many coding-call lines enter one digest; the rest are counted.
const MAX_CODING_DIGEST_LINES: usize = 64;

/// Longest path a coding digest line carries. A path is model-supplied and
/// otherwise unbounded; the cap keeps one call from crowding out the rest.
const MAX_DIGEST_PATH_BYTES: usize = 256;

/// A tool call whose finding lives in the private owner checkpoint rather than
/// in a SQL digest line. Their digest lines carry a path and numbers only —
/// never file content, old/new strings or command output — so these calls
/// contribute no arbitrary prior text.
fn is_coding_tool(name: &str) -> bool {
    matches!(
        name,
        "read" | "write" | "edit" | "delete" | "follow_up_child"
    )
}

/// Whether `digest` may stand in for `evicted`: it must record at least one
/// finding whenever the evicted slice held a recordable call. A digest whose
/// body came back empty (no room for the header) or which holds only its
/// header would have replaced real observations with nothing, so the caller
/// keeps the groups instead and retries under the hard budget, which has the
/// room to record them.
pub(super) fn records_evicted_findings(digest: &ChatMessage, evicted: &[ChatMessage]) -> bool {
    if !is_digest(digest) {
        return false;
    }
    digest.content.len() > DIGEST_HEADER.len()
        || !evicted
            .iter()
            .flat_map(|message| &message.tool_calls)
            .any(|call| recordable_coding_call(call) || !is_coding_tool(&call.name))
}

/// Whether a coding call can produce a digest line: one of the four file
/// operations, with a path that carries no credential-shaped value. This is
/// the cheap half of [`coding_line`], so counting omitted calls never parses
/// a result.
fn recordable_coding_call(call: &crate::ToolCall) -> bool {
    matches!(call.name.as_str(), "read" | "write" | "edit" | "delete")
        && call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| saya_types::redact(path) == path)
}

/// The tool result for `call_id`, when the evicted slice still holds it and it
/// parses as JSON. Only the keys named below are read; a row value, file
/// content or error body is never copied into a line.
fn result_value(evicted: &[ChatMessage], call_id: &str) -> Option<Value> {
    let content = evicted
        .iter()
        .find(|message| message.role == "tool" && message.tool_call_id.as_deref() == Some(call_id))?
        .content
        .as_str();
    serde_json::from_str(content).ok()
}

/// The path a coding line may carry: model-supplied, so truncated at a UTF-8
/// boundary to [`MAX_DIGEST_PATH_BYTES`].
fn digest_path(path: &str) -> &str {
    &path[..super::tools::floor_boundary(path, MAX_DIGEST_PATH_BYTES)]
}

/// The line range a `read` call observed, from its result when the result
/// reports one and from its arguments otherwise. `None` means unranged, or a
/// range that cannot be stated without inventing an end.
fn read_range(arguments: &Value, result: Option<&Value>) -> Option<(u64, u64)> {
    let observed = result.and_then(|value| {
        let start = value.get("start_line").and_then(Value::as_u64)?;
        let returned = value.get("lines_returned").and_then(Value::as_u64)?;
        Some((start, start.saturating_add(returned).saturating_sub(1)))
    });
    observed.or_else(|| {
        let start = arguments.get("start_line").and_then(Value::as_u64)?;
        let count = arguments.get("line_count").and_then(Value::as_u64)?;
        Some((start, start.saturating_add(count).saturating_sub(1)))
    })
}

/// The replacement count an `edit` reported, falling back to the call's own
/// shape when it failed before reporting one.
fn edit_replacements(arguments: &Value, result: Option<&Value>) -> Option<u64> {
    result
        .and_then(|value| value.get("replacements").and_then(Value::as_u64))
        .or_else(|| {
            arguments
                .get("edits")
                .and_then(Value::as_array)
                .map(|edits| edits.len() as u64)
        })
        .or_else(|| arguments.get("old_text").and_then(Value::as_str).map(|_| 1))
}

/// One digest line for an evicted coding call: its path and numbers only.
/// `follow_up_child` stays excluded; a call with no path records nothing.
/// A failed call keeps its line and is marked `(failed)`.
fn coding_line(call: &crate::ToolCall, evicted: &[ChatMessage]) -> Option<String> {
    if !recordable_coding_call(call) {
        return None;
    }
    let path = digest_path(call.arguments.get("path").and_then(Value::as_str)?);
    let result = result_value(evicted, &call.id);
    let failed = result
        .as_ref()
        .is_some_and(|value| value.get("error").is_some());
    let mut line = match call.name.as_str() {
        "read" => match read_range(&call.arguments, result.as_ref()) {
            Some((start, end)) => format!("read {path} lines {start}-{end}"),
            None => format!("read {path}"),
        },
        "write" => {
            let bytes = result
                .as_ref()
                .and_then(|value| value.get("size").and_then(Value::as_u64))
                .or_else(|| {
                    call.arguments
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|content| content.len() as u64)
                });
            match bytes {
                Some(bytes) => format!("wrote {path} ({bytes} bytes)"),
                None => format!("wrote {path}"),
            }
        }
        "edit" => match edit_replacements(&call.arguments, result.as_ref()) {
            Some(count) => format!("edited {path} ({count} replacements)"),
            None => format!("edited {path}"),
        },
        "delete" => format!("deleted {path}"),
        _ => return None,
    };
    if failed {
        line.push_str(" (failed)");
    }
    Some(line)
}

/// Builds one `assistant`-role digest message summarizing the tool-result
/// groups in `evicted`. The digest is the assistant's own prior findings,
/// compressed to statements and result shapes — never row values — so it
/// belongs on the assistant turn, not in the user (untrusted-data) channel.
///
/// `evicted` is the contiguous slice being replaced. SQL findings come from
/// its actual calls/results; cumulative coding facts come from private owner
/// observations, never parsed assistant text. At most one digest survives.
#[cfg(test)]
pub(super) fn build_digest(evicted: &[ChatMessage]) -> ChatMessage {
    build_digest_with_checkpoint(
        evicted,
        &super::coding_checkpoint::CodingCheckpoint::default(),
        16 * 1024,
    )
}

pub(super) fn build_digest_with_checkpoint(
    evicted: &[ChatMessage],
    checkpoint: &super::coding_checkpoint::CodingCheckpoint,
    allowance: usize,
) -> ChatMessage {
    let allowance = allowance.min(16 * 1024);
    let mut body = DIGEST_HEADER.to_owned();
    let coding = checkpoint.render(allowance.saturating_sub(body.len()));
    if !coding.is_empty() {
        body.push_str(&coding);
        if body.len() < allowance {
            body.push('\n');
        }
    }
    // Coding calls record a path and numbers only. They are collected before
    // the SQL lines so a digest that must drop findings drops the older SQL
    // statements, not the file trail the next turn needs.
    let mut coding_lines: Vec<String> = Vec::new();
    let mut more_coding = 0usize;
    for message in evicted {
        for call in &message.tool_calls {
            if !recordable_coding_call(call) {
                continue;
            }
            if coding_lines.len() < MAX_CODING_DIGEST_LINES {
                if let Some(line) = coding_line(call, evicted) {
                    coding_lines.push(line);
                }
            } else {
                more_coding += 1;
            }
        }
    }
    for (index, line) in coding_lines.iter().enumerate() {
        if body.len().saturating_add(line.len() + 1) > allowance {
            more_coding += coding_lines.len() - index;
            break;
        }
        body.push_str(line);
        body.push('\n');
    }
    if more_coding > 0 {
        let line = format!("… and {more_coding} more tool calls");
        if body.len().saturating_add(line.len() + 1) <= allowance {
            body.push_str(&line);
            body.push('\n');
        }
    }
    let mut count = 0;
    for message in evicted {
        for call in &message.tool_calls {
            if count >= 64 {
                break;
            }
            // Coding lines were recorded above; a coding call is never a SQL
            // finding, even when a refused call carries SQL-shaped arguments.
            if is_coding_tool(&call.name) {
                continue;
            }
            let statement = call.arguments.get("sql").and_then(Value::as_str);
            let line = digest_line(statement, result_shape(evicted, &call.id));
            if body.len().saturating_add(line.len() + 1) > allowance {
                break;
            }
            body.push_str(&line);
            body.push('\n');
            count += 1;
        }
    }
    if body.len() > allowance {
        body.clear();
    }
    ChatMessage {
        role: "assistant".into(),
        content: body,
        tool_calls: Vec::new(),
        tool_call_id: None,
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    }
}

pub(super) fn checkpoint_size(
    checkpoint: &super::coding_checkpoint::CodingCheckpoint,
    allowance: usize,
) -> usize {
    let coding = checkpoint.render(allowance);
    if coding.is_empty() {
        0
    } else {
        DIGEST_HEADER.len() + coding.len() + 1 + "assistant".len()
    }
}

/// The shape of a tool result, parsed from the `tool` message whose
/// `tool_call_id` matches `call_id`: the row count and the column names. Row
/// values are never read. `None` fields cover a truncated result (its content
/// is no longer valid JSON), an error result, or a non-row tool.
struct ResultShape {
    row_count: Option<u64>,
    columns: Option<Vec<String>>,
}

fn result_shape(evicted: &[ChatMessage], call_id: &str) -> Option<ResultShape> {
    let value = result_value(evicted, call_id)?;
    let row_count = value.get("row_count").and_then(Value::as_u64);
    let columns = value
        .get("columns")
        .and_then(Value::as_array)
        .filter(|array| {
            array.len() <= 128
                && array.iter().all(|item| {
                    item.as_str()
                        .is_some_and(|name| name.len() <= 256 && saya_types::redact(name) == name)
                })
        })
        .map(|array| {
            array
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        });
    Some(ResultShape { row_count, columns })
}

/// One digest line for a tool call: the statement that ran and the shape of
/// its result, or an explicit "unavailable" when no shape could be parsed
/// (the result was truncated, an error, or a non-row tool). The statement is
/// debug-quoted so a statement containing newlines or brackets cannot escape
/// the line's delimiters.
fn digest_line(statement: Option<&str>, shape: Option<ResultShape>) -> String {
    let safe = saya_types::redact(statement.unwrap_or("(no sql statement)"));
    let stmt = if safe.len() > 2048 {
        format!(
            "{}...[truncated SQL]",
            &safe[..super::tools::floor_boundary(&safe, 2048)]
        )
    } else {
        safe
    };
    match shape {
        Some(ResultShape {
            row_count: Some(rows),
            columns: Some(names),
        }) if !names.is_empty() => {
            format!(
                "- statement: {stmt:?}; rows: {rows}; columns: [{}]",
                names.join(", ")
            )
        }
        Some(ResultShape {
            row_count: Some(rows),
            columns: _,
        }) => format!("- statement: {stmt:?}; rows: {rows}"),
        _ => format!("- statement: {stmt:?}; (result shape unavailable)"),
    }
}

#[cfg(test)]
mod tests;
