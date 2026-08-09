mod box_render;
mod markdown;
mod query;
mod shared;
#[cfg(test)]
mod tests;

use saya_types::QueryResult;

pub(crate) use markdown::format_markdown_tables;
pub(crate) use query::format_table;

/// Renders an EXPLAIN result as readable plan text WITHOUT the box-table's 40-char
/// column cap. A single-column plan (PostgreSQL/DuckDB, one plan line per row) prints
/// one line per row; a multi-column plan (MySQL) prints `column: value` lines per row,
/// with a blank line between rows.
pub(crate) fn format_plan(result: &QueryResult) -> String {
    if result.rows.is_empty() {
        return "(no plan rows)".to_string();
    }

    let col_count = result.columns.len();
    if col_count <= 1 {
        let fetch_count = col_count.max(1);
        result
            .rows
            .iter()
            .map(|row| {
                let cells = normalize_row(row, fetch_count);
                cell_to_string(&cells[0])
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        result
            .rows
            .iter()
            .map(|row| {
                let cells = normalize_row(row, col_count);
                result
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(i, col_name)| {
                        let val_str = cell_to_string(&cells[i]);
                        format!("{col_name}: {val_str}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

fn normalize_row(row: &serde_json::Value, col_count: usize) -> Vec<serde_json::Value> {
    let mut cells = match row {
        serde_json::Value::Array(arr) => arr.clone(),
        scalar => vec![scalar.clone()],
    };
    cells.resize(col_count, serde_json::Value::Null);
    cells
}

fn cell_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_plan_single_column_long_line() {
        let long_line = "Seq Scan on film  (cost=0.00..64.00 rows=1000 width=15) blah blah blah extra text that exceeds forty characters";
        let result = QueryResult {
            columns: vec!["QUERY PLAN".to_string()],
            rows: vec![json!([long_line]), json!(["Filter: (film_id > 10)"])],
            row_count: 2,
            truncated: false,
            executed_sql: "EXPLAIN SELECT * FROM film".to_string(),
        };

        let output = format_plan(&result);

        assert!(output.contains(long_line));
        assert!(!output.contains('…'));
        assert!(!output.contains('┌'));
        assert!(!output.contains('│'));
        assert!(!output.contains('└'));

        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], long_line);
        assert_eq!(lines[1], "Filter: (film_id > 10)");
    }

    #[test]
    fn test_format_plan_multi_column() {
        let result = QueryResult {
            columns: vec!["id".to_string(), "type".to_string()],
            rows: vec![json!([1, "ALL"])],
            row_count: 1,
            truncated: false,
            executed_sql: "EXPLAIN SELECT * FROM film".to_string(),
        };

        let output = format_plan(&result);

        assert!(output.contains("id: 1"));
        assert!(output.contains("type: ALL"));
    }

    #[test]
    fn test_format_plan_empty_rows() {
        let result = QueryResult {
            columns: vec!["QUERY PLAN".to_string()],
            rows: vec![],
            row_count: 0,
            truncated: false,
            executed_sql: "EXPLAIN SELECT * FROM film".to_string(),
        };

        let output = format_plan(&result);

        assert_eq!(output, "(no plan rows)");
    }
}
