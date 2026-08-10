use saya_types::QueryResult;
use serde_json::Value;

pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn bounded_result(
    columns: Vec<String>,
    mut rows: Vec<Value>,
    max_rows: usize,
    executed_sql: String,
) -> QueryResult {
    let truncated = rows.len() > max_rows;
    rows.truncate(max_rows);
    QueryResult {
        row_count: rows.len(),
        columns,
        rows,
        truncated,
        executed_sql,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_to_hex() {
        assert_eq!(bytes_to_hex(&[0, 255, 16]), "00ff10");
        assert_eq!(bytes_to_hex(&[]), "");
    }

    #[test]
    fn test_bounded_result_truncation() {
        let cols = vec!["id".to_string()];
        let rows = vec![Value::from(1), Value::from(2), Value::from(3)];
        let res = bounded_result(cols.clone(), rows, 2, "SELECT 1".to_string());
        assert_eq!(res.row_count, 2);
        assert!(res.truncated);
        assert_eq!(res.rows, vec![Value::from(1), Value::from(2)]);
        assert_eq!(res.executed_sql, "SELECT 1");

        let rows_not_truncated = vec![Value::from(1), Value::from(2)];
        let res2 = bounded_result(cols, rows_not_truncated, 2, "SELECT 2".to_string());
        assert_eq!(res2.row_count, 2);
        assert!(!res2.truncated);
        assert_eq!(res2.executed_sql, "SELECT 2");
    }
}
