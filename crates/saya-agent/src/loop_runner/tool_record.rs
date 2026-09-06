//! Extracts the persisted record of a tool call from its request and result:
//! the arguments the model sent (which carry the SQL) and the value-free
//! shape of a query result (row count + column names). Result rows — cell
//! values — are never copied; only `row_count` and the `columns` names are
//! read, so a planted cell value cannot reach a persisted session.
//!
//! This is the agent-side half of "let a session reproduce its own run": the
//! loop builds a [`crate::ToolMetadata`] per call here, the CLI's `record_turn`
//! maps it onto the store's redacted record, and a resumed session can show
//! what ran without the live event stream.

use serde_json::Value;

use crate::ToolResultShape;

/// Builds the value-free shape of a serialized query result, or `None` when
/// `result` is not a row-shaped query result. Reads only the `row_count` and
/// `columns` keys; `rows` is never inspected, so no cell value can escape.
///
/// A query result carries `columns` as an array of **strings** (the column
/// names) — that is what distinguishes a `QueryResult` from another tool's
/// JSON (e.g. `render_chart`'s `{path, note}` or the `result_shape` tool's own
/// `{columns: [{name, type}]}`). When `columns` is absent, not an array, or
/// not all strings, the result is treated as non-row-shaped and `None` is
/// returned. An empty result still names its columns, so a zero-row query
/// yields `Some` with the names and `row_count: 0`.
pub(super) fn result_shape_of(result: &Value) -> Option<ToolResultShape> {
    let row_count = result.get("row_count")?.as_u64()?;
    let columns = result.get("columns")?.as_array()?;
    // A query result carries column names as strings; a non-query result that
    // happens to carry a `row_count` key (none today) or object-typed columns
    // (the `result_shape` tool's own output) is left as `None` so it is not
    // mistaken for a row-shaped query result.
    if !columns.iter().all(Value::is_string) {
        return None;
    }
    let columns = columns
        .iter()
        .map(|name| name.as_str().unwrap_or("").to_owned())
        .collect();
    Some(ToolResultShape { row_count, columns })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query result with rows yields its row count and column names, and the
    /// cell values in `rows` never reach the shape.
    #[test]
    fn result_shape_of_takes_count_and_names_and_no_cell_values() {
        let result = serde_json::json!({
            "columns": ["customer", "age"],
            "rows": [["SECRET_CELL_9f3a", 30]],
            "row_count": 1,
            "truncated": false,
            "executed_sql": "SELECT customer, age FROM t"
        });
        let shape = result_shape_of(&result).expect("a query result has a shape");
        assert_eq!(shape.row_count, 1);
        assert_eq!(shape.columns, vec!["customer", "age"]);
        let serialized = serde_json::to_string(&shape).unwrap();
        assert!(
            !serialized.contains("SECRET_CELL_9f3a"),
            "a cell value reached the result shape: {serialized}"
        );
    }

    /// An empty result keeps its column names with a zero count.
    #[test]
    fn result_shape_of_keeps_column_names_for_an_empty_result() {
        let result = serde_json::json!({
            "columns": ["customer", "age"],
            "rows": [],
            "row_count": 0,
            "truncated": false
        });
        let shape = result_shape_of(&result).expect("an empty result still has a shape");
        assert_eq!(shape.row_count, 0);
        assert_eq!(shape.columns, vec!["customer", "age"]);
    }

    /// A non-query result (no `row_count`, or non-string columns) yields
    /// `None`, so a tool that did not run a query records no shape.
    #[test]
    fn result_shape_of_is_none_for_a_non_row_shaped_result() {
        assert!(result_shape_of(&serde_json::json!({"path": "/tmp/x", "note": "ok"})).is_none());
        assert!(result_shape_of(&serde_json::json!({"error": "rejected"})).is_none());
        // The `result_shape` tool's own output carries columns as objects,
        // not strings — not a row-shaped query result.
        assert!(
            result_shape_of(&serde_json::json!({
                "row_count": 1,
                "columns": [{"name": "x", "type": "TEXT"}]
            }))
            .is_none()
        );
    }
}
