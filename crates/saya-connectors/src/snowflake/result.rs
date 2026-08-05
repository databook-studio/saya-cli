use saya_types::{ConnectionError, QueryResult};
use serde_json::Value;

use super::errors;

pub(crate) fn result(
    body: &Value,
    max_rows: usize,
    original: String,
) -> Result<QueryResult, ConnectionError> {
    let data = body.get("data").unwrap_or(body);
    let rows = data
        .as_array()
        .or_else(|| {
            data.get("rowset")
                .or_else(|| data.get("data"))
                .and_then(Value::as_array)
        })
        .ok_or_else(errors::query)?;
    let meta = body.get("resultSetMetaData").unwrap_or(data);
    let columns = meta
        .get("rowType")
        .or_else(|| {
            meta.get("resultSetMetaData")
                .and_then(|item| item.get("rowType"))
        })
        .or_else(|| data.get("rowtype"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    bounded(columns, rows.clone(), max_rows, original)
}

pub(crate) fn bounded(
    columns: Vec<String>,
    mut rows: Vec<Value>,
    max_rows: usize,
    original: String,
) -> Result<QueryResult, ConnectionError> {
    if max_rows == 0 {
        return Err(errors::query());
    }
    let truncated = rows.len() > max_rows;
    rows.truncate(max_rows);
    Ok(QueryResult {
        row_count: rows.len(),
        columns,
        rows,
        truncated,
        executed_sql: original,
    })
}

pub(crate) fn chunk_rows(text: &str) -> Result<Vec<Value>, ConnectionError> {
    let body = text.trim().trim_end_matches(',').trim();
    if body.is_empty() {
        return Ok(vec![]);
    }
    serde_json::from_str(&format!("[{body}]")).map_err(|_| errors::query())
}

#[cfg(test)]
mod tests {
    use super::{bounded, chunk_rows};
    use serde_json::json;

    #[test]
    fn parses_unbracketed_rows_and_truncates() {
        assert_eq!(
            chunk_rows("[1],[2],\n").unwrap(),
            vec![json!([1]), json!([2])]
        );
        assert!(
            bounded(vec![], vec![json!([1]), json!([2])], 1, "SELECT 1".into())
                .unwrap()
                .truncated
        );
    }
}
