//! Unit tests for the scope-line shaper: wording, truncation, and the
//! no-connection / no-query edges. Completion-level behaviour (capture at
//! dispatch, retention after a profile change, clip parity, copy) lives in
//! `scope_tests.rs`, which drives `sql_task::complete`.

use super::scope::{SCOPE_SQL_CHARS, scope_line, with_scope_line};

#[test]
fn names_connection_and_query() {
    assert_eq!(
        scope_line(Some("sales-demo"), "select region from orders"),
        Some("from sales-demo · select region from orders".to_string())
    );
}

#[test]
fn no_connection_means_no_from_prefix() {
    assert_eq!(
        scope_line(None, "select region from orders"),
        Some("select region from orders".to_string())
    );
    assert_eq!(
        scope_line(Some(""), "select 1"),
        Some("select 1".to_string())
    );
}

#[test]
fn empty_query_leaves_connection_or_nothing() {
    assert_eq!(
        scope_line(Some("sales-demo"), ""),
        Some("from sales-demo".to_string())
    );
    assert_eq!(scope_line(None, ""), None);
    assert_eq!(scope_line(None, "   \n "), None);
}

#[test]
fn a_long_query_is_truncated_with_an_ellipsis() {
    let long = format!("select {} from orders", "region, ".repeat(30));
    let line = scope_line(Some("sales-demo"), &long).expect("a line is yielded");
    assert!(
        line.starts_with("from sales-demo · "),
        "connection prefix survives truncation: {line:?}"
    );
    assert!(
        line.chars().count() <= "from sales-demo · ".chars().count() + SCOPE_SQL_CHARS,
        "query part capped at {SCOPE_SQL_CHARS} chars: {line:?}"
    );
    assert!(line.ends_with('…'), "the cut is marked: {line:?}");
    assert!(
        !line.contains('\n'),
        "one display line, never a wrapped query: {line:?}"
    );
}

#[test]
fn multiline_sql_collapses_to_one_line() {
    let line = scope_line(
        Some("sales-demo"),
        "select region,\n  count(*) from orders\nwhere x > 1",
    )
    .expect("a line is yielded");
    assert_eq!(
        line,
        "from sales-demo · select region, count(*) from orders where x > 1"
    );
}

#[test]
fn with_scope_line_appends_after_the_footer() {
    let table = "┌──┐\n└──┘\n1 row(s)".to_string();
    let out = with_scope_line(
        table.clone(),
        Some("sales-demo"),
        "select region from orders",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[lines.len() - 2], "1 row(s)");
    assert_eq!(
        lines[lines.len() - 1],
        "from sales-demo · select region from orders"
    );
}

#[test]
fn with_scope_line_appends_nothing_when_there_is_nothing_to_say() {
    let table = "┌──┐\n└──┘\n1 row(s)".to_string();
    assert_eq!(with_scope_line(table.clone(), None, ""), table);
}
