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

use crate::ChatMessage;
use serde_json::Value;

/// Leading sentinel marking a message as a compaction digest, so a later
/// compaction can identify and re-evict the prior digest (keeping at most one
/// digest in the conversation). Tool results are JSON (`{...}`) and history
/// assistant turns do not begin with this marker, so the prefix is unambiguous.
pub(super) const DIGEST_SENTINEL: &str = "…[compacted:";

/// Whether `message` is a compaction digest produced by an earlier compaction.
pub(super) fn is_digest(message: &ChatMessage) -> bool {
    message.content.starts_with(DIGEST_SENTINEL)
}

/// Builds one `assistant`-role digest message summarizing the tool-result
/// groups in `evicted`. The digest is the assistant's own prior findings,
/// compressed to statements and result shapes — never row values — so it
/// belongs on the assistant turn, not in the user (untrusted-data) channel.
///
/// `evicted` is the contiguous slice of messages being replaced (old tool
/// groups and any prior digest). Only the tool calls and their matching
/// `tool` results contribute; a prior digest in the slice has no tool calls
/// and is silently dropped rather than re-summarized, so at most one digest
/// survives a compaction.
pub(super) fn build_digest(evicted: &[ChatMessage]) -> ChatMessage {
    let mut lines = Vec::new();
    for message in evicted {
        for call in &message.tool_calls {
            let statement = call.arguments.get("sql").and_then(Value::as_str);
            let shape = result_shape(evicted, &call.id);
            lines.push(digest_line(statement, shape));
        }
    }
    let body = if lines.is_empty() {
        format!("{DIGEST_SENTINEL} no prior tool results]")
    } else {
        format!(
            "{DIGEST_SENTINEL} {} prior tool result(s) summarized]\n{}",
            lines.len(),
            lines.join("\n")
        )
    };
    ChatMessage {
        role: "assistant".into(),
        content: body,
        tool_calls: Vec::new(),
        tool_call_id: None,
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
    let content = evicted
        .iter()
        .find(|message| message.role == "tool" && message.tool_call_id.as_deref() == Some(call_id))?
        .content
        .as_str();
    let value: Value = serde_json::from_str(content).ok()?;
    let row_count = value.get("row_count").and_then(Value::as_u64);
    let columns = value.get("columns").and_then(Value::as_array).map(|array| {
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
    let stmt = statement.unwrap_or("(no sql statement)");
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
mod tests {
    use super::*;
    use crate::ToolCall;

    fn assistant_with_call(id: &str, sql: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: id.into(),
                name: "bounded_sql_query".into(),
                arguments: serde_json::json!({"sql": sql}),
            }],
            tool_call_id: None,
        }
    }

    fn tool_result(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(id.into()),
        }
    }

    /// The digest names each evicted statement and reports the result's row
    /// count and column names — the shape, not the rows.
    #[test]
    fn digest_names_statements_and_shapes_without_row_values() {
        let result = serde_json::json!({
            "columns": ["id", "name"],
            "rows": [["a", "PLANTED_CELL_VALUE"], ["b", "c"]],
            "row_count": 2,
            "truncated": false,
            "executed_sql": "SELECT id, name FROM t"
        });
        let evicted = vec![
            assistant_with_call("c1", "SELECT id, name FROM t"),
            tool_result("c1", &serde_json::to_string(&result).unwrap()),
        ];
        let digest = build_digest(&evicted);
        assert!(is_digest(&digest));
        let body = &digest.content;
        assert!(
            body.contains("SELECT id, name FROM t"),
            "digest must name the statement: {body}"
        );
        assert!(
            body.contains("rows: 2"),
            "digest must report the row count: {body}"
        );
        assert!(
            body.contains("columns: [id, name]"),
            "digest must report column names: {body}"
        );
        assert!(
            !body.contains("PLANTED_CELL_VALUE"),
            "a row value must never appear in the digest: {body}"
        );
    }

    /// A truncated result (its content is no longer valid JSON) is recorded as
    /// "result shape unavailable" — the statement is still named.
    #[test]
    fn digest_records_unavailable_shape_for_an_unparseable_result() {
        let evicted = vec![
            assistant_with_call("c1", "SELECT 1"),
            tool_result("c1", "{\"columns\":[\"a\"…[truncated]"),
        ];
        let digest = build_digest(&evicted);
        assert!(digest.content.contains("SELECT 1"));
        assert!(digest.content.contains("result shape unavailable"));
    }

    /// A non-row tool (no `sql`, no `row_count`/`columns`) still produces a
    /// line, naming the absence of a statement.
    #[test]
    fn digest_handles_a_call_with_no_sql_argument() {
        let evicted = vec![ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "schema_discovery".into(),
                arguments: serde_json::json!({}),
            }],
            tool_call_id: None,
        }];
        let digest = build_digest(&evicted);
        assert!(digest.content.contains("no sql statement"));
    }
}
