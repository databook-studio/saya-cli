use saya_types::QueryResult;
use serde_json::Value;

use crate::common::{MAX_RESULT_BYTES, cap_cell, value_bytes};

/// The `jobs.query` request body. `maxResults` is set one above the row cap so
/// a result that fills the cap but has more rows reports `pageToken` and is
/// marked truncated, while `maximumBytesBilled` is the server-side cost bound
/// that fails the job if the query scans more than the configured allowance.
pub(crate) fn query_body(sql: &str, max_rows: usize, cap: u64, location: Option<&str>) -> Value {
    let mut body = serde_json::json!({
        "query": sql,
        "useLegacySql": false,
        "maxResults": max_rows.saturating_add(1),
        "maximumBytesBilled": cap.to_string(),
    });
    if let Some(location) = location {
        body["location"] = Value::String(location.into());
    }
    body
}

/// The `jobs.insert` body for a dry-run. BigQuery estimates the bytes the
/// query would scan without running it, which lets the connector refuse an
/// over-budget query before it executes.
pub(crate) fn dry_run_body(sql: &str, cap: u64, location: Option<&str>) -> Value {
    let mut query = serde_json::json!({
        "query": sql,
        "useLegacySql": false,
        "maximumBytesBilled": cap.to_string(),
    });
    if let Some(location) = location {
        query["location"] = Value::String(location.into());
    }
    serde_json::json!({
        "configuration": {
            "dryRun": true,
            "query": query,
        }
    })
}

/// Turns a `jobs.query` response into a bounded `QueryResult`. `schema.fields`
/// names the columns in select order; `rows[].f[].v` carries the cell values.
/// Rows are accumulated up to the row cap and the shared byte budget, setting
/// `truncated` when either bound stops the loop or when a `pageToken` reports
/// more rows remain.
pub(crate) fn parse_result(value: Value, max_rows: usize, original_sql: String) -> QueryResult {
    let columns = value
        .get("schema")
        .and_then(|s| s.get("fields"))
        .and_then(Value::as_array)
        .map(|fields| {
            fields
                .iter()
                .map(|field| {
                    field
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let rows = value.get("rows").and_then(Value::as_array).cloned();
    let page_token = value.get("pageToken").is_some();

    let mut collected = Vec::new();
    let mut result_bytes = 0;
    let mut truncated = page_token;
    if let Some(rows) = rows {
        for row in rows {
            if collected.len() == max_rows {
                truncated = true;
                break;
            }
            let cells = row
                .get("f")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            // BigQuery rows are positional: one `f` entry per column. Pad a
            // short row with null so every result row matches the column count
            // rather than silently dropping trailing columns.
            let values: Vec<Value> = (0..columns.len())
                .map(|index| {
                    cells
                        .get(index)
                        .and_then(|cell| cell.get("v").cloned())
                        .unwrap_or(Value::Null)
                })
                .map(cap_cell)
                .collect();
            for cell in &values {
                result_bytes += value_bytes(cell);
            }
            collected.push(Value::Array(values));
            if result_bytes > MAX_RESULT_BYTES {
                truncated = true;
                break;
            }
        }
    }
    QueryResult {
        row_count: collected.len(),
        columns,
        rows: collected,
        truncated,
        executed_sql: original_sql,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_body_carries_byte_cap_and_row_cap_as_strings() {
        let body = query_body("SELECT 1", 10, 1024, None);
        assert_eq!(body["query"], "SELECT 1");
        assert_eq!(body["useLegacySql"], false);
        // maxResults is one above the row cap so truncation is detectable.
        assert_eq!(body["maxResults"], 11);
        // BigQuery's int64 fields are formatted as strings.
        assert_eq!(body["maximumBytesBilled"], "1024");
        assert!(body.get("location").is_none());
    }

    #[test]
    fn query_body_includes_location_when_set() {
        let body = query_body("SELECT 1", 5, 1024, Some("EU"));
        assert_eq!(body["location"], "EU");
    }

    #[test]
    fn dry_run_body_marks_dry_run_and_carries_byte_cap() {
        let body = dry_run_body("SELECT 1", 2048, None);
        assert_eq!(body["configuration"]["dryRun"], true);
        assert_eq!(body["configuration"]["query"]["query"], "SELECT 1");
        assert_eq!(body["configuration"]["query"]["maximumBytesBilled"], "2048");
    }

    #[test]
    fn parse_result_reads_columns_and_rows() {
        let body = json!({
            "schema": {"fields": [{"name": "id", "type": "INTEGER"}, {"name": "name", "type": "STRING"}]},
            "rows": [{"f": [{"v": "1"}, {"v": "a"}]}, {"f": [{"v": "2"}, {"v": "b"}]}]
        });
        let result = parse_result(body, 10, "SELECT id, name FROM t".into());
        assert_eq!(result.columns, vec!["id", "name"]);
        assert_eq!(result.row_count, 2);
        assert!(!result.truncated);
        assert_eq!(result.rows[0], json!(["1", "a"]));
        assert_eq!(result.rows[1], json!(["2", "b"]));
    }

    #[test]
    fn parse_result_keeps_columns_when_there_are_no_rows() {
        let body = json!({"schema": {"fields": [{"name": "id"}]}, "rows": []});
        let result = parse_result(body, 10, "SELECT id FROM empty".into());
        assert_eq!(result.columns, vec!["id"]);
        assert_eq!(result.row_count, 0);
        assert!(!result.truncated);
    }

    #[test]
    fn parse_result_caps_rows_and_marks_truncated() {
        let rows: Vec<Value> = (0..5)
            .map(|i| json!({"f": [{"v": i.to_string()}]}))
            .collect();
        let body = json!({"schema": {"fields": [{"name": "id"}]}, "rows": rows});
        let result = parse_result(body, 3, "SELECT id FROM t".into());
        assert_eq!(result.row_count, 3);
        assert!(result.truncated);
    }

    #[test]
    fn parse_result_marks_truncated_on_page_token() {
        let body = json!({
            "schema": {"fields": [{"name": "id"}]},
            "rows": [{"f": [{"v": "1"}]}],
            "pageToken": "next"
        });
        let result = parse_result(body, 10, "SELECT id FROM t".into());
        assert_eq!(result.row_count, 1);
        assert!(result.truncated);
    }

    #[test]
    fn parse_result_missing_cell_becomes_null() {
        let body = json!({
            "schema": {"fields": [{"name": "a"}, {"name": "b"}]},
            "rows": [{"f": [{"v": "1"}]}]
        });
        let result = parse_result(body, 10, "SELECT a, b FROM t".into());
        assert_eq!(result.rows[0], json!(["1", null]));
    }

    #[test]
    fn parse_result_marks_truncated_on_byte_budget() {
        let cell = "x".repeat(512 * 1024);
        let rows: Vec<Value> = (0..40)
            .map(|_| json!({"f": [{"v": cell.clone()}]}))
            .collect();
        let body = json!({"schema": {"fields": [{"name": "payload"}]}, "rows": rows});
        let result = parse_result(body, 100, "SELECT payload FROM t".into());
        assert!(result.truncated);
        assert!(result.row_count < 40);
    }
}
