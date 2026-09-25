use super::*;
use crate::interactive::tui::table::format_table;
use saya_types::QueryResult;

fn wide_result() -> QueryResult {
    QueryResult {
        columns: vec![
            "id".into(),
            "name".into(),
            "status".into(),
            "region".into(),
            "country".into(),
            "city".into(),
            "postal".into(),
            "carrier".into(),
            "tracking".into(),
            "total".into(),
            "tax".into(),
            "discount".into(),
            "currency".into(),
            "placed_at".into(),
            "shipped_at".into(),
        ],
        rows: vec![serde_json::json!([
            1,
            "alice",
            "fulfilled",
            "north",
            "CA",
            "SF",
            "94105",
            "UPS",
            "1Z999",
            120,
            12,
            5,
            "USD",
            "2024-01-01",
            "2024-01-03"
        ])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT * FROM orders".into(),
    }
}

fn block_lines(result: &QueryResult) -> Vec<String> {
    format_table(result).lines().map(str::to_string).collect()
}

fn view(h_offset: usize, pin_first: bool, columns: Option<Vec<&str>>) -> WideTableView {
    WideTableView {
        h_offset,
        pin_first,
        columns: columns.map(|c| c.into_iter().map(String::from).collect()),
    }
}

#[test]
fn a_narrow_view_shows_the_first_columns_and_the_footer() {
    let lines = block_lines(&wide_result());
    let wv = view(0, false, None);
    let out = clip_table_block(&lines, &wv, 20);
    // The footer line is preserved verbatim.
    assert!(
        out.iter().any(|l| l.contains("1 row(s)")),
        "footer must survive clipping: {out:?}"
    );
    // Column 0 (id) is visible at the left; a later column is not.
    let header = out
        .iter()
        .find(|l| l.starts_with('│') && l.contains("id"))
        .expect("header row present");
    assert!(header.contains("id"), "first column shows: {header}");
    assert!(
        !header.contains("shipped_at"),
        "last column does not fit in 20 cols: {header}"
    );
}

#[test]
fn scrolling_right_reveals_later_columns() {
    let lines = block_lines(&wide_result());
    let left = clip_table_block(&lines, &view(0, false, None), 24);
    let left_header = left
        .iter()
        .find(|l| l.contains("name"))
        .expect("header at offset 0");
    assert!(
        !left_header.contains("shipped_at"),
        "offset 0 must not show the last column: {left_header}"
    );

    // Push the offset far enough that the early columns leave the window.
    let right = clip_table_block(&lines, &view(14, false, None), 24);
    let right_header = right
        .iter()
        .find(|l| l.starts_with('│'))
        .expect("header at offset 14");
    assert!(
        right_header.contains("shipped_at"),
        "offset 14 must reveal the last column: {right_header}"
    );
    assert!(
        !right_header.contains("│ id"),
        "offset 14 must drop the first column: {right_header}"
    );
}

#[test]
fn pinning_keeps_the_first_column_while_scrolling() {
    let lines = block_lines(&wide_result());
    let out = clip_table_block(&lines, &view(14, true, None), 28);
    let header = out
        .iter()
        .find(|l| l.starts_with('│'))
        .expect("header present");
    assert!(
        header.contains("│ id"),
        "pinned first column stays put: {header}"
    );
    assert!(
        header.contains("shipped_at"),
        "a far column is reached by scrolling: {header}"
    );
    assert!(
        !header.contains("name"),
        "a column between the pin and the scroll window is hidden: {header}"
    );
}

#[test]
fn column_filter_keeps_only_named_columns() {
    let lines = block_lines(&wide_result());
    let out = clip_table_block(&lines, &view(0, false, Some(vec!["id", "total"])), 80);
    let header = out
        .iter()
        .find(|l| l.starts_with('│'))
        .expect("header present");
    assert!(header.contains("id"), "selected column id shows: {header}");
    assert!(
        header.contains("total"),
        "selected column total shows: {header}"
    );
    assert!(
        !header.contains("name"),
        "unselected column is hidden: {header}"
    );
    assert!(
        !header.contains("carrier"),
        "unselected column is hidden: {header}"
    );
}

#[test]
fn a_column_filter_that_matches_nothing_falls_back_to_all() {
    let lines = block_lines(&wide_result());
    let out = clip_table_block(&lines, &view(0, false, Some(vec!["nope"])), 80);
    let header = out
        .iter()
        .find(|l| l.starts_with('│'))
        .expect("header present");
    assert!(
        header.contains("name"),
        "no match falls back to all columns: {header}"
    );
}

#[test]
fn clipping_preserves_line_count() {
    let lines = block_lines(&wide_result());
    let wv = view(5, true, None);
    let out = clip_table_block(&lines, &wv, 30);
    assert_eq!(out.len(), lines.len(), "clipping never adds or drops lines");
}

#[test]
fn a_non_box_block_is_hard_clipped_not_reconstructed() {
    let lines = vec!["(no columns) — 2 row(s)".to_string()];
    let out = clip_table_block(&lines, &view(0, false, None), 10);
    assert_eq!(out[0].chars().count(), 10);
    assert!(out[0].starts_with("(no column"));
}
