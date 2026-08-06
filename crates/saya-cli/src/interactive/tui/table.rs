use saya_types::QueryResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alignment {
    Left,
    Right,
}

/// Renders headers + string rows as an aligned box-drawing table (the lines between
/// the top border and the bottom border, inclusive — NO footer). `numeric[i] == true`
/// right-aligns column i's data cells; headers are always left-aligned. Column widths
/// are capped at 40 with ellipsis truncation, matching the existing behaviour.
pub(crate) fn render_box(
    headers: &[String],
    rows: &[Vec<String>],
    numeric: &[bool],
) -> Vec<String> {
    let num_cols = headers.len();
    if num_cols == 0 {
        return Vec::new();
    }

    let mut col_widths = Vec::with_capacity(num_cols);
    for (i, col_name) in headers.iter().enumerate() {
        let header_len = col_name.chars().count();
        let max_cell_len = rows
            .iter()
            .map(|r| r.get(i).map_or(0, |cell| cell.chars().count()))
            .max()
            .unwrap_or(0);
        let w = header_len.max(max_cell_len).min(40);
        col_widths.push(w);
    }

    let mut lines = Vec::with_capacity(rows.len() + 4);

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
        hdr.push_str(&format_cell(&headers[i], w, Alignment::Left));
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
    for row in rows {
        let mut line = String::from("│");
        for (i, &w) in col_widths.iter().enumerate() {
            let align = if numeric.get(i).copied().unwrap_or(false) {
                Alignment::Right
            } else {
                Alignment::Left
            };
            let cell_str = row.get(i).map(|s| s.as_str()).unwrap_or("");
            line.push(' ');
            line.push_str(&format_cell(cell_str, w, align));
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

    lines
}

/// Formats a query result as an aligned, box-drawn table (monospace text) for
/// the transcript. Columns are padded to their content width (capped), long
/// cells are truncated with an ellipsis, and a row-count footer is appended.
/// Numeric columns are right-aligned; text and header cells are left-aligned.
pub(crate) fn format_table(result: &QueryResult) -> String {
    let num_cols = result.columns.len();
    let row_count = result.rows.len();
    let trunc_suffix = if result.truncated { " (truncated)" } else { "" };

    if num_cols == 0 {
        return format!("(no columns) — {row_count} row(s){trunc_suffix}");
    }

    let numeric: Vec<bool> = (0..num_cols)
        .map(|col_idx| {
            let mut has_numeric = false;
            let mut all_non_null_numeric = true;

            for row_val in &result.rows {
                if let Some(cell) = get_cell(row_val, col_idx) {
                    match cell {
                        serde_json::Value::Null => {}
                        serde_json::Value::Number(_) => {
                            has_numeric = true;
                        }
                        _ => {
                            all_non_null_numeric = false;
                        }
                    }
                }
            }

            has_numeric && all_non_null_numeric
        })
        .collect();

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

    let mut lines = render_box(&result.columns, &rows, &numeric);
    lines.push(format!("{row_count} row(s){trunc_suffix}"));
    lines.join("\n")
}

/// Rewrites any GitHub-style Markdown pipe tables found in `text` into box-drawing tables
/// (the same style as query results), leaving all other lines untouched. A table is only
/// recognised when a header row `| a | b |` is immediately followed by a separator row whose
/// cells are all dashes/colons (e.g. `|---|:--:|`) — so stray `|` in prose is never mangled.
#[allow(dead_code)]
pub(crate) fn format_markdown_tables(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut output = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len() {
            let header_opt = parse_line_raw_cells(lines[i]);
            let sep_opt = parse_line_raw_cells(lines[i + 1]);
            match (header_opt, sep_opt) {
                (Some(header_raw), Some(sep_raw)) if is_separator_row(&sep_raw) => {
                    let header: Vec<String> = header_raw.iter().map(|c| clean_cell(c)).collect();
                    let num_cols = header.len();
                    i += 2;

                    let mut data_rows = Vec::new();
                    while i < lines.len() {
                        if let Some(raw_cells) = parse_line_raw_cells(lines[i]) {
                            let mut cells: Vec<String> =
                                raw_cells.iter().map(|c| clean_cell(c)).collect();
                            if cells.len() < num_cols {
                                cells.resize(num_cols, String::new());
                            } else {
                                cells.truncate(num_cols);
                            }
                            data_rows.push(cells);
                            i += 1;
                        } else {
                            break;
                        }
                    }

                    let numeric: Vec<bool> = (0..num_cols)
                        .map(|col_idx| {
                            let mut has_non_empty = false;
                            let mut all_f64 = true;
                            for row in &data_rows {
                                if let Some(cell_str) = row.get(col_idx) {
                                    let cell = cell_str.trim();
                                    if !cell.is_empty() {
                                        has_non_empty = true;
                                        if cell.parse::<f64>().is_err() {
                                            all_f64 = false;
                                        }
                                    }
                                }
                            }
                            has_non_empty && all_f64
                        })
                        .collect();

                    let box_lines = render_box(&header, &data_rows, &numeric);
                    output.extend(box_lines);
                    continue;
                }
                _ => {}
            }
        }

        output.push(lines[i].to_string());
        i += 1;
    }

    output.join("\n")
}

fn parse_line_raw_cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let mut fields: Vec<&str> = line.split('|').map(str::trim).collect();
    if fields.first() == Some(&"") {
        fields.remove(0);
    }
    if fields.last() == Some(&"") {
        fields.pop();
    }
    if fields.is_empty() {
        None
    } else {
        Some(fields.into_iter().map(String::from).collect())
    }
}

fn is_separator_row(cells: &[String]) -> bool {
    if cells.is_empty() {
        return false;
    }
    cells.iter().all(|c| is_separator_cell(c))
}

fn is_separator_cell(cell: &str) -> bool {
    let cell = cell.trim();
    if cell.is_empty() {
        return false;
    }
    let mut s = cell;
    if s.starts_with(':') {
        s = &s[1..];
    }
    if s.ends_with(':') {
        s = &s[..s.len() - 1];
    }
    !s.is_empty() && s.chars().all(|c| c == '-')
}

fn clean_cell(cell: &str) -> String {
    let mut s = cell.trim();
    if s.starts_with("**") && s.ends_with("**") && s.len() >= 4 {
        s = &s[2..s.len() - 2];
        s = s.trim();
    }
    if s.starts_with('`') && s.ends_with('`') && s.len() >= 2 {
        s = &s[1..s.len() - 1];
        s = s.trim();
    }
    if s.starts_with("**") && s.ends_with("**") && s.len() >= 4 {
        s = &s[2..s.len() - 2];
        s = s.trim();
    }
    s.to_string()
}

fn get_cell(row_val: &serde_json::Value, col_idx: usize) -> Option<&serde_json::Value> {
    match row_val {
        serde_json::Value::Array(vals) => vals.get(col_idx),
        other => {
            if col_idx == 0 {
                Some(other)
            } else {
                None
            }
        }
    }
}

fn cell_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "NULL".to_string(),
        other => other.to_string(),
    }
}

fn format_cell(text: &str, col_width: usize, alignment: Alignment) -> String {
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
    match alignment {
        Alignment::Left => format!("{content}{padding}"),
        Alignment::Right => format!("{padding}{content}"),
    }
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
    fn test_format_table_right_aligns_numeric_columns() {
        let result = QueryResult {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                serde_json::json!([1, "alice"]),
                serde_json::json!([100, "bob"]),
            ],
            row_count: 2,
            truncated: false,
            executed_sql: "".into(),
        };

        let formatted = format_table(&result);
        let lines: Vec<&str> = formatted.lines().collect();

        assert_eq!(lines[1], "│ id  │ name  │");
        assert_eq!(lines[3], "│   1 │ alice │");
        assert_eq!(lines[4], "│ 100 │ bob   │");
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

    #[test]
    fn test_format_markdown_tables_basic() {
        let input = "Here are the films:\n\n| Rank | Title | Count |\n|------|-------|-------|\n| 1 | BUCKET | 34 |\n| 2 | ROCKETEER | 33 |\n\nDone.";
        let output = format_markdown_tables(input);

        assert!(!output.contains("|------|"));
        assert!(output.contains('┌'));
        assert!(output.contains('│'));
        assert!(output.contains('└'));
        assert!(output.contains("BUCKET"));
        assert!(output.contains("34"));
        assert!(output.starts_with("Here are the films:\n\n┌"));
        assert!(output.ends_with("\n\nDone."));
    }

    #[test]
    fn test_format_markdown_tables_numeric_alignment() {
        let input = "| Rank | Title | Count |\n|------|-------|-------|\n| 1 | BUCKET | 34 |\n| 2 | ROCKETEER | 33 |";
        let output = format_markdown_tables(input);
        let lines: Vec<&str> = output.lines().collect();

        // Count and Rank are numeric and right-padded (leading spaces inside cell), Title is text and left-padded
        let bucket_line = lines
            .iter()
            .find(|l| l.contains("BUCKET"))
            .expect("data row with BUCKET");
        let rocketeer_line = lines
            .iter()
            .find(|l| l.contains("ROCKETEER"))
            .expect("data row with ROCKETEER");

        assert_eq!(*bucket_line, "│    1 │ BUCKET    │    34 │");
        assert_eq!(*rocketeer_line, "│    2 │ ROCKETEER │    33 │");
    }

    #[test]
    fn test_format_markdown_tables_bold_and_backticks_stripped() {
        let input = "| Header |\n|---|\n| **BUCKET** |\n| `id` |";
        let output = format_markdown_tables(input);

        assert!(output.contains("BUCKET"));
        assert!(!output.contains("**BUCKET**"));
        assert!(output.contains("id"));
        assert!(!output.contains("`id`"));
    }

    #[test]
    fn test_format_markdown_tables_no_table() {
        let input = "This is a line with | a pipe in prose.\nAnother line without pipes.";
        let output = format_markdown_tables(input);

        assert_eq!(output, input);
    }
}
