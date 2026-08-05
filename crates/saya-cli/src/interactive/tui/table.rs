use saya_types::QueryResult;

/// Formats a query result as an aligned, box-drawn table (monospace text) for
/// the transcript. Columns are padded to their content width (capped), long
/// cells are truncated with an ellipsis, and a row-count footer is appended.
pub(crate) fn format_table(result: &QueryResult) -> String {
    let num_cols = result.columns.len();
    let row_count = result.rows.len();
    let trunc_suffix = if result.truncated { " (truncated)" } else { "" };

    if num_cols == 0 {
        return format!("(no columns) — {row_count} row(s){trunc_suffix}");
    }

    let rows: Vec<Vec<String>> = result
        .rows
        .iter()
        .map(|row_val| {
            let mut cells = match row_val {
                serde_json::Value::Array(vals) => vals.iter().map(cell_text).collect::<Vec<_>>(),
                other => vec![cell_text(other)],
            };
            if cells.len() < num_cols {
                cells.resize(num_cols, String::new());
            } else {
                cells.truncate(num_cols);
            }
            cells
        })
        .collect();

    let mut col_widths = Vec::with_capacity(num_cols);
    for (i, col_name) in result.columns.iter().enumerate() {
        let header_len = col_name.chars().count();
        let max_cell_len = rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0);
        let w = header_len.max(max_cell_len).min(40);
        col_widths.push(w);
    }

    let mut lines = Vec::with_capacity(rows.len() + 5);

    // Top border
    let mut top = String::from("┌");
    for (i, &w) in col_widths.iter().enumerate() {
        top.push_str(&"─".repeat(w + 2));
        if i + 1 < num_cols {
            top.push('┬');
        } else {
            top.push('┐');
        }
    }
    lines.push(top);

    // Header row
    let mut hdr = String::from("│");
    for (i, &w) in col_widths.iter().enumerate() {
        hdr.push(' ');
        hdr.push_str(&format_cell(&result.columns[i], w));
        hdr.push(' ');
        hdr.push('│');
    }
    lines.push(hdr);

    // Header separator
    let mut sep = String::from("├");
    for (i, &w) in col_widths.iter().enumerate() {
        sep.push_str(&"─".repeat(w + 2));
        if i + 1 < num_cols {
            sep.push('┼');
        } else {
            sep.push('┤');
        }
    }
    lines.push(sep);

    // Data rows
    for row in &rows {
        let mut line = String::from("│");
        for (i, &w) in col_widths.iter().enumerate() {
            line.push(' ');
            line.push_str(&format_cell(&row[i], w));
            line.push(' ');
            line.push('│');
        }
        lines.push(line);
    }

    // Bottom border
    let mut bot = String::from("└");
    for (i, &w) in col_widths.iter().enumerate() {
        bot.push_str(&"─".repeat(w + 2));
        if i + 1 < num_cols {
            bot.push('┴');
        } else {
            bot.push('┘');
        }
    }
    lines.push(bot);

    // Footer
    lines.push(format!("{row_count} row(s){trunc_suffix}"));

    lines.join("\n")
}

fn cell_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "NULL".to_string(),
        other => other.to_string(),
    }
}

fn format_cell(text: &str, col_width: usize) -> String {
    if col_width == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    let content = if char_count > col_width {
        let take_len = col_width.saturating_sub(1);
        let mut s: String = text.chars().take(take_len).collect();
        s.push('…');
        s
    } else {
        text.to_string()
    };
    let content_len = content.chars().count();
    let padding = " ".repeat(col_width.saturating_sub(content_len));
    format!("{content}{padding}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::QueryResult;

    #[test]
    fn test_format_table_basic_alignment() {
        let result = QueryResult {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                serde_json::json!([1, "alice"]),
                serde_json::json!([2, "bob"]),
            ],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT * FROM users".into(),
        };

        let formatted = format_table(&result);
        let lines: Vec<&str> = formatted.lines().collect();

        assert_eq!(lines.len(), 7);
        assert!(lines[0].starts_with('┌'));
        assert!(lines[1].contains("id"));
        assert!(lines[1].contains("name"));
        assert!(lines[3].contains("alice"));
        assert!(lines[4].contains("bob"));
        assert_eq!(lines[6], "2 row(s)");

        let box_lines = &lines[0..6];
        let first_len = box_lines[0].chars().count();
        for line in box_lines {
            assert_eq!(line.chars().count(), first_len);
        }
    }

    #[test]
    fn test_format_table_null_rendering() {
        let result = QueryResult {
            columns: vec!["val".into()],
            rows: vec![serde_json::json!([serde_json::Value::Null])],
            row_count: 1,
            truncated: false,
            executed_sql: "".into(),
        };

        let formatted = format_table(&result);
        assert!(formatted.contains("NULL"));
    }

    #[test]
    fn test_format_table_truncation_long_cell() {
        let long_val = "a".repeat(50);
        let result = QueryResult {
            columns: vec!["col".into()],
            rows: vec![serde_json::json!([long_val])],
            row_count: 1,
            truncated: false,
            executed_sql: "".into(),
        };

        let formatted = format_table(&result);
        assert!(formatted.contains('…'));
    }

    #[test]
    fn test_format_table_ragged_row() {
        let result = QueryResult {
            columns: vec!["c1".into(), "c2".into()],
            rows: vec![serde_json::json!([1])],
            row_count: 1,
            truncated: false,
            executed_sql: "".into(),
        };

        let formatted = format_table(&result);
        let lines: Vec<&str> = formatted.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[5], "1 row(s)");
    }

    #[test]
    fn test_format_table_empty_columns() {
        let result = QueryResult {
            columns: vec![],
            rows: vec![serde_json::json!([1]), serde_json::json!([2])],
            row_count: 2,
            truncated: false,
            executed_sql: "".into(),
        };

        let formatted = format_table(&result);
        assert_eq!(formatted, "(no columns) — 2 row(s)");
    }

    #[test]
    fn test_format_table_footer_truncated() {
        let result = QueryResult {
            columns: vec!["id".into()],
            rows: vec![serde_json::json!([1])],
            row_count: 1,
            truncated: true,
            executed_sql: "".into(),
        };

        let formatted = format_table(&result);
        assert!(formatted.ends_with("1 row(s) (truncated)"));

        let empty_cols = QueryResult {
            columns: vec![],
            rows: vec![serde_json::json!([1])],
            row_count: 1,
            truncated: true,
            executed_sql: "".into(),
        };
        let formatted_empty = format_table(&empty_cols);
        assert_eq!(formatted_empty, "(no columns) — 1 row(s) (truncated)");
    }
}
