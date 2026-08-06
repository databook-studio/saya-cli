use super::{format_markdown_tables, format_table};
use saya_types::QueryResult;

#[test]
fn test_format_table_basic_alignment() {
    let result = QueryResult {
        columns: vec!["id".into(), "name".into()],
        rows: vec![serde_json::json!([1, "alice"]), serde_json::json!([2, "bob"])],
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
        rows: vec![serde_json::json!([1, "alice"]), serde_json::json!([100, "bob"])],
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
    let input =
        "| Rank | Title | Count |\n|------|-------|-------|\n| 1 | BUCKET | 34 |\n| 2 | ROCKETEER | 33 |";
    let output = format_markdown_tables(input);
    let lines: Vec<&str> = output.lines().collect();

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
