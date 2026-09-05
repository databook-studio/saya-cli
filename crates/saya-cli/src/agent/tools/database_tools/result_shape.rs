//! The `result_shape` tool's body: run a bounded read-only query through the
//! same path as `bounded_sql_query` and return its shape — row count, whether
//! the row cap was hit, and the column names with an inferred type label — and
//! never any cell value.
//!
//! The shape is built only from the result's `row_count`, `truncated`, column
//! names, and the JSON *kind* of each cell in the first row (string → `TEXT`,
//! integer → `INTEGER`, &c.). A kind test (`value.is_string()`) inspects no
//! value content, so a planted sentinel can never reach the output.

use serde_json::{Value, json};

use saya_agent::ToolError;

use super::DatabaseTools;
use crate::connection::ConnectionEntry;

impl DatabaseTools {
    /// Runs one bounded read-only SQL query and returns its shape, reusing the
    /// `bounded_sql_query` execution path so the safety layer, bounds, and
    /// error mapping are identical. A statement the read-only policy refuses is
    /// refused here by the same path, not a second one.
    pub(super) async fn result_shape(
        &self,
        entry: &ConnectionEntry,
        sql: &str,
    ) -> Result<Value, ToolError> {
        // A1: detect a confirmed claim this statement contradicts, independent
        // of whether the query then succeeds. Best-effort: a missing receipt/log
        // or a fail-closed detector records nothing; the turn is never failed by
        // detection.
        self.detect_and_record_overrides(sql, entry.dialect);
        let result = crate::agent::state_tools::query(
            entry.connector.as_ref(),
            sql,
            self.max_rows,
            self.state_db.as_ref(),
            entry.profile_id.as_deref(),
        )
        .await?;
        Ok(shape_of(&result))
    }
}

/// Builds the value-free shape of a serialized [`saya_types::QueryResult`]:
/// `row_count`, `truncated`, and `columns` (name + inferred type label). No
/// cell value is copied — only the count, the cap flag, the column names, and a
/// type label derived from each first-row cell's JSON kind, which carries no
/// value content. When there are no rows the type label is `null`, but the
/// column names remain so an empty result is still self-describing.
fn shape_of(result: &Value) -> Value {
    let row_count = result.get("row_count").and_then(Value::as_u64).unwrap_or(0);
    let truncated = result
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let first_row = result
        .get("rows")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(Value::as_array);
    let columns = result
        .get("columns")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let ty = first_row
                        .and_then(|cells| cells.get(index))
                        .map(value_type_label)
                        .unwrap_or(Value::Null);
                    json!({ "name": name, "type": ty })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "row_count": row_count,
        "truncated": truncated,
        "columns": columns,
    })
}

/// A declared-style type label for one JSON value, inferred from its kind alone.
fn value_type_label(value: &Value) -> Value {
    let label = match value {
        Value::Null => "NULL",
        Value::Bool(_) => "BOOLEAN",
        Value::Number(n) if n.is_i64() || n.is_u64() => "INTEGER",
        Value::Number(_) => "REAL",
        Value::String(_) => "TEXT",
        Value::Array(_) | Value::Object(_) => "JSON",
    };
    Value::String(label.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_of_infers_types_from_the_first_row_and_omits_values() {
        let result = json!({
            "columns": ["customer", "age", "balance", "active", "meta"],
            "rows": [["Alice", 30, 99.5, true, {"k": 1}]],
            "row_count": 1,
            "truncated": false,
            "executed_sql": "SELECT 1"
        });
        let shape = shape_of(&result);
        assert_eq!(shape["row_count"], 1);
        assert_eq!(shape["truncated"], false);
        assert_eq!(shape["columns"][0]["name"], "customer");
        assert_eq!(shape["columns"][0]["type"], "TEXT");
        assert_eq!(shape["columns"][1]["type"], "INTEGER");
        assert_eq!(shape["columns"][2]["type"], "REAL");
        assert_eq!(shape["columns"][3]["type"], "BOOLEAN");
        assert_eq!(shape["columns"][4]["type"], "JSON");
        let serialized = serde_json::to_string(&shape).unwrap();
        assert!(!serialized.contains("Alice"), "no cell value may escape");
    }

    #[test]
    fn shape_of_keeps_column_names_with_null_types_for_an_empty_result() {
        let result = json!({
            "columns": ["customer", "age"],
            "rows": [],
            "row_count": 0,
            "truncated": false,
            "executed_sql": "SELECT 1"
        });
        let shape = shape_of(&result);
        assert_eq!(shape["row_count"], 0);
        let columns = shape["columns"].as_array().unwrap();
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0]["name"], "customer");
        assert_eq!(columns[0]["type"], Value::Null);
        assert_eq!(columns[1]["name"], "age");
        assert_eq!(columns[1]["type"], Value::Null);
    }
}
