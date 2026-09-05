//! The `column_health` tool's body: run a bounded read-only query through the
//! same path as `bounded_sql_query` and report per-column health statistics —
//! null count, null percentage, distinct value count, and numeric zero count —
//! and never any cell value.
//!
//! The stats are built only from the result's `row_count`, `truncated`, column
//! names, and per-cell JSON equality (for distinct counts). Distinct values are
//! counted by their serialized form, which carries no readable content into the
//! output, so a planted sentinel can never reach the returned JSON.

use std::collections::HashSet;

use serde_json::{Value, json};

use saya_agent::ToolError;

use super::DatabaseTools;
use crate::connection::ConnectionEntry;

impl DatabaseTools {
    /// Runs one bounded read-only SQL query and returns per-column health
    /// statistics, reusing the `bounded_sql_query` execution path so the safety
    /// layer, bounds, and error mapping are identical.
    pub(super) async fn column_health(
        &self,
        entry: &ConnectionEntry,
        sql: &str,
    ) -> Result<Value, ToolError> {
        self.detect_and_record_overrides(sql, entry.dialect);
        let result = crate::agent::state_tools::query(
            entry.connector.as_ref(),
            sql,
            self.max_rows,
            self.state_db.as_ref(),
            entry.profile_id.as_deref(),
        )
        .await?;
        Ok(health_of(&result))
    }
}

/// Builds the value-free column health report from a serialized
/// [`saya_types::QueryResult`]: `row_count`, `truncated`, and per-column
/// `nulls`, `null_pct`, `distinct`, and `zeros`. No cell value is copied —
/// distinct values are counted by their serialized form, which is never placed
/// in the output. NULL counts as one distinct value.
fn health_of(result: &Value) -> Value {
    let row_count = result.get("row_count").and_then(Value::as_u64).unwrap_or(0);
    let truncated = result
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let empty: Vec<Value> = Vec::new();
    let rows = result
        .get("rows")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let columns = result
        .get("columns")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let column_stats = columns
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let mut nulls: u64 = 0;
            let mut zeros: u64 = 0;
            let mut distinct: HashSet<String> = HashSet::new();
            for row in rows {
                let cell = row.as_array().and_then(|cells| cells.get(index));
                match cell {
                    None | Some(Value::Null) => {
                        nulls += 1;
                        distinct.insert("\u{0}null".to_string());
                    }
                    Some(value) => {
                        if is_numeric_zero(value) {
                            zeros += 1;
                        }
                        distinct.insert(serde_json::to_string(value).unwrap_or_default());
                    }
                }
            }
            let null_pct = if row_count > 0 {
                nulls as f64 / row_count as f64 * 100.0
            } else {
                0.0
            };
            json!({
                "name": name,
                "nulls": nulls,
                "null_pct": null_pct,
                "distinct": distinct.len(),
                "zeros": zeros,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "row_count": row_count,
        "truncated": truncated,
        "columns": column_stats,
    })
}

/// True when `value` is a JSON number equal to zero (integer 0 or float 0.0).
fn is_numeric_zero(value: &Value) -> bool {
    value.as_f64().is_some_and(|f| f == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_of_reports_nulls_distinct_and_zeros_without_values() {
        let result = json!({
            "columns": ["day", "amount", "flag"],
            "rows": [
                [null, 0, "a"],
                [null, 5, "a"],
                [null, 0, "b"]
            ],
            "row_count": 3,
            "truncated": false,
            "executed_sql": "SELECT 1"
        });
        let health = health_of(&result);
        assert_eq!(health["row_count"], 3);
        assert_eq!(health["truncated"], false);
        let columns = health["columns"].as_array().unwrap();
        assert_eq!(columns[0]["name"], "day");
        assert_eq!(columns[0]["nulls"], 3);
        assert_eq!(columns[0]["null_pct"], 100.0);
        assert_eq!(columns[0]["distinct"], 1);
        assert_eq!(columns[0]["zeros"], 0);
        assert_eq!(columns[1]["name"], "amount");
        assert_eq!(columns[1]["nulls"], 0);
        assert_eq!(columns[1]["null_pct"], 0.0);
        assert_eq!(columns[1]["distinct"], 2);
        assert_eq!(columns[1]["zeros"], 2);
        assert_eq!(columns[2]["name"], "flag");
        assert_eq!(columns[2]["nulls"], 0);
        assert_eq!(columns[2]["distinct"], 2);
        assert_eq!(columns[2]["zeros"], 0);
    }

    #[test]
    fn health_of_never_leaks_a_planted_cell_value() {
        let result = json!({
            "columns": ["secret"],
            "rows": [["HEALTH_LEAK_SENTINEL_99"]],
            "row_count": 1,
            "truncated": false,
            "executed_sql": "SELECT 1"
        });
        let health = health_of(&result);
        let serialized = serde_json::to_string(&health).unwrap();
        assert!(
            !serialized.contains("HEALTH_LEAK_SENTINEL_99"),
            "a cell value escaped into the health report: {serialized}"
        );
        assert!(serialized.contains("secret"));
        assert!(serialized.contains("\"distinct\":1"));
    }

    #[test]
    fn health_of_handles_empty_result() {
        let result = json!({
            "columns": ["a", "b"],
            "rows": [],
            "row_count": 0,
            "truncated": false,
            "executed_sql": "SELECT 1"
        });
        let health = health_of(&result);
        assert_eq!(health["row_count"], 0);
        let columns = health["columns"].as_array().unwrap();
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0]["nulls"], 0);
        assert_eq!(columns[0]["null_pct"], 0.0);
        assert_eq!(columns[0]["distinct"], 0);
        assert_eq!(columns[0]["zeros"], 0);
    }
}
