//! Renders a query result as a text horizontal bar chart for the transcript.
use saya_types::QueryResult;

const BAR_WIDTH: usize = 40; // longest bar in cells
const MAX_ROWS: usize = 20; // rows charted before truncating

/// Renders `result` as a horizontal bar chart: a label column and a numeric value
/// column, one bar per row. Returns Err when there is no numeric column to plot.
pub(crate) fn format_bar_chart(result: &QueryResult) -> Result<String, String> {
    let col_count = result.columns.len();
    if col_count == 0 {
        return Err("no numeric column to chart — try a query that returns a number".into());
    }

    let normalized_rows: Vec<Vec<serde_json::Value>> = result
        .rows
        .iter()
        .map(|row| normalize_row(row, col_count))
        .collect();

    // Determine, per column, whether it is NUMERIC:
    // it has >= 1 non-null cell and EVERY non-null cell is a serde_json::Value::Number.
    // value_col = index of the first numeric column.
    let mut value_col = None;
    for col_idx in 0..col_count {
        let mut non_null_count = 0;
        let mut all_numbers = true;
        for row in &normalized_rows {
            let cell = &row[col_idx];
            if !cell.is_null() {
                non_null_count += 1;
                if !cell.is_number() {
                    all_numbers = false;
                    break;
                }
            }
        }
        if non_null_count >= 1 && all_numbers {
            value_col = Some(col_idx);
            break;
        }
    }

    let value_col = match value_col {
        Some(idx) => idx,
        None => {
            return Err("no numeric column to chart — try a query that returns a number".into());
        }
    };

    // label_col = the first column index that is not value_col
    // (prefer any column != value_col; if the result has only one column, use value_col itself)
    let label_col = (0..col_count)
        .find(|&c| c != value_col)
        .unwrap_or(value_col);

    let value_col_name = result
        .columns
        .get(value_col)
        .map(|s| s.as_str())
        .unwrap_or("");
    let label_col_name = result
        .columns
        .get(label_col)
        .map(|s| s.as_str())
        .unwrap_or("");

    let charted_rows = normalized_rows.iter().take(MAX_ROWS);

    struct RowData {
        label_raw: String,
        value_f64: f64,
        value_str: String,
    }

    let mut row_data_list = Vec::with_capacity(MAX_ROWS.min(normalized_rows.len()));
    let mut max_value_f64: f64 = 0.0;
    let mut max_label_len: usize = 0;

    for row in charted_rows {
        let label_cell = &row[label_col];
        let value_cell = &row[value_col];

        let label_raw = cell_to_string(label_cell);
        let value_f64 = value_cell.as_f64().unwrap_or(0.0);
        let value_str = cell_to_string(value_cell);

        let label_len = label_raw.chars().count();
        if label_len > max_label_len {
            max_label_len = label_len;
        }

        if value_f64.max(0.0) > max_value_f64 {
            max_value_f64 = value_f64.max(0.0);
        }

        row_data_list.push(RowData {
            label_raw,
            value_f64,
            value_str,
        });
    }

    // max_value = the maximum of value.max(0.0) across charted rows; if 0.0, set to 1.0
    let max_value = if max_value_f64 == 0.0 {
        1.0
    } else {
        max_value_f64
    };

    // label_width = min(24, max label char count across charted rows), at least 1
    let label_width = max_label_len.clamp(1, 24);

    let mut lines = Vec::new();
    lines.push(format!("chart: {value_col_name} by {label_col_name}"));

    for row in row_data_list {
        let truncated_label = truncate_label(&row.label_raw, label_width);
        let val_pos = row.value_f64.max(0.0);
        let bar_len = ((val_pos / max_value) * BAR_WIDTH as f64).round() as usize;
        let bar = "█".repeat(bar_len);

        lines.push(format!(
            "{truncated_label:<label_width$} │ {bar} {}",
            row.value_str
        ));
    }

    if result.rows.len() > MAX_ROWS {
        lines.push(format!(
            "… (showing first {MAX_ROWS} of {} rows)",
            result.rows.len()
        ));
    }

    Ok(lines.join("\n"))
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

fn truncate_label(label: &str, label_width: usize) -> String {
    let char_count = label.chars().count();
    if char_count <= label_width {
        label.to_string()
    } else if label_width <= 1 {
        "…".to_string()
    } else {
        let mut truncated: String = label.chars().take(label_width - 1).collect();
        truncated.push('…');
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_chart_text_label_and_numeric() {
        let result = QueryResult {
            columns: vec!["city".to_string(), "pop".to_string()],
            rows: vec![
                json!(["Tokyo", 37.4]),
                json!(["Delhi", 29.3]),
                json!(["Shanghai", 26.3]),
            ],
            row_count: 3,
            truncated: false,
            executed_sql: "SELECT city, pop FROM cities".to_string(),
        };
        let chart = format_bar_chart(&result).unwrap();
        assert!(chart.contains("chart: pop by city"));
        assert!(chart.contains('█'));
        assert!(chart.contains("Tokyo"));
        assert!(chart.contains("Delhi"));
        assert!(chart.contains("Shanghai"));

        // Largest value ("Tokyo" with 37.4) should have the longest bar (40 '█' chars)
        let tokyo_line = chart.lines().find(|l| l.contains("Tokyo")).unwrap();
        let tokyo_bar_count = tokyo_line.chars().filter(|&c| c == '█').count();
        assert_eq!(tokyo_bar_count, 40);

        let delhi_line = chart.lines().find(|l| l.contains("Delhi")).unwrap();
        let delhi_bar_count = delhi_line.chars().filter(|&c| c == '█').count();
        assert!(delhi_bar_count < 40);
    }

    #[test]
    fn test_chart_no_numeric_column() {
        let result = QueryResult {
            columns: vec!["name".to_string(), "city".to_string()],
            rows: vec![json!(["Alice", "Tokyo"]), json!(["Bob", "Delhi"])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT name, city FROM users".to_string(),
        };
        let err = format_bar_chart(&result).unwrap_err();
        assert_eq!(
            err,
            "no numeric column to chart — try a query that returns a number"
        );
    }

    #[test]
    fn test_chart_value_label_formatting() {
        let result = QueryResult {
            columns: vec!["item".to_string(), "price".to_string()],
            rows: vec![json!(["apple", 4.99]), json!(["banana", 0.5])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT item, price FROM items".to_string(),
        };
        let chart = format_bar_chart(&result).unwrap();
        assert!(chart.contains("4.99"));
        assert!(chart.contains("0.5"));
    }

    #[test]
    fn test_chart_single_numeric_column() {
        let result = QueryResult {
            columns: vec!["score".to_string()],
            rows: vec![json!([10]), json!([20])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT score FROM scores".to_string(),
        };
        let chart = format_bar_chart(&result).unwrap();
        assert!(chart.contains("chart: score by score"));
        assert!(chart.contains("10"));
        assert!(chart.contains("20"));
    }

    #[test]
    fn test_chart_truncation_footer() {
        let mut rows = Vec::new();
        for i in 0..25 {
            rows.push(json!([format!("label_{i}"), i]));
        }
        let result = QueryResult {
            columns: vec!["label".to_string(), "val".to_string()],
            rows,
            row_count: 25,
            truncated: false,
            executed_sql: "SELECT label, val FROM test".to_string(),
        };
        let chart = format_bar_chart(&result).unwrap();
        assert!(chart.contains("… (showing first 20 of 25 rows)"));
    }
}
