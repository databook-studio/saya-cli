use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::Value;

use super::{ClickHouseConnector, diagnose, errors};
use crate::common::{MAX_RESULT_BYTES, cap_cell, value_bytes};

pub(crate) async fn query(
    connector: &ClickHouseConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_clickhouse_sql(&request.sql, request.max_rows)?;
    let response = connector.post(&sql, request.max_rows).await?;
    if !response.status().is_success() {
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();
        return Err(diagnose::query_failure(status, &headers, &body));
    }
    let value: Value = response.json().await.map_err(errors::body)?;
    Ok(parse_result(value, request.max_rows, request.sql))
}

/// Turns a ClickHouse `FORMAT JSON` body into a bounded `QueryResult`.
///
/// `meta` carries the column names in select order; `data` is one JSON object
/// per row keyed by those names. Rows are accumulated up to `max_rows` and the
/// shared byte budget, setting `truncated` when either bound stops the loop.
/// The server's own `max_result_rows` has already capped the result, so this
/// client cap is a second bound, not a replacement for it.
fn parse_result(value: Value, max_rows: usize, original_sql: String) -> QueryResult {
    let columns = value
        .get("meta")
        .and_then(Value::as_array)
        .map(|meta| {
            meta.iter()
                .map(|column| {
                    column
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let rows = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut collected = Vec::new();
    let mut result_bytes = 0;
    let mut truncated = false;
    for row in rows {
        if collected.len() == max_rows {
            truncated = true;
            break;
        }
        let values: Vec<Value> = columns
            .iter()
            .map(|name| row.get(name).cloned().unwrap_or(Value::Null))
            .map(cap_cell)
            .collect();
        for value in &values {
            result_bytes += value_bytes(value);
        }
        collected.push(Value::Array(values));
        if result_bytes > MAX_RESULT_BYTES {
            truncated = true;
            break;
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
    fn parse_result_reads_columns_in_meta_order() {
        let body = json!({
            "meta": [
                {"name": "id", "type": "UInt64"},
                {"name": "name", "type": "String"}
            ],
            "data": [
                {"id": 1, "name": "a"},
                {"id": 2, "name": "b"}
            ],
            "rows": 2
        });
        let result = parse_result(body, 10, "SELECT id, name FROM t".into());
        assert_eq!(result.columns, vec!["id", "name"]);
        assert_eq!(result.row_count, 2);
        assert!(!result.truncated);
        assert_eq!(result.rows[0], json!([1, "a"]));
        assert_eq!(result.rows[1], json!([2, "b"]));
    }

    #[test]
    fn parse_result_keeps_columns_when_there_are_no_rows() {
        // Unlike newline-delimited formats, `FORMAT JSON` reports `meta` even
        // for an empty result, so the columns survive a zero-row query.
        let body = json!({
            "meta": [{"name": "id", "type": "UInt64"}],
            "data": [],
            "rows": 0
        });
        let result = parse_result(body, 10, "SELECT id FROM empty".into());
        assert_eq!(result.columns, vec!["id"]);
        assert_eq!(result.row_count, 0);
        assert!(!result.truncated);
    }

    #[test]
    fn parse_result_caps_rows_client_side_and_marks_truncated() {
        let mut data = Vec::new();
        for i in 0..5 {
            data.push(json!({"id": i}));
        }
        let body = json!({
            "meta": [{"name": "id", "type": "UInt64"}],
            "data": data,
            "rows": 5
        });
        let result = parse_result(body, 3, "SELECT id FROM t".into());
        assert_eq!(result.row_count, 3);
        assert!(result.truncated);
    }

    #[test]
    fn parse_result_marks_truncated_on_byte_budget() {
        // Each cell is under the per-cell cap, so `cap_cell` leaves it whole;
        // the 16 MiB result budget is what stops the loop, mirroring the
        // streaming connectors. Forty 512 KiB rows is 20 MiB, well past it.
        let cell = "x".repeat(512 * 1024);
        let data: Vec<Value> = (0..40).map(|_| json!({"payload": cell})).collect();
        let body = json!({
            "meta": [{"name": "payload", "type": "String"}],
            "data": data,
            "rows": 40
        });
        let result = parse_result(body, 100, "SELECT payload FROM t".into());
        assert!(result.truncated);
        assert!(result.row_count < 40);
    }

    #[test]
    fn parse_result_missing_row_key_becomes_null() {
        let body = json!({
            "meta": [{"name": "a"}, {"name": "b"}],
            "data": [{"a": 1}]
        });
        let result = parse_result(body, 10, "SELECT a, b FROM t".into());
        assert_eq!(result.rows[0], json!([1, null]));
    }
}
