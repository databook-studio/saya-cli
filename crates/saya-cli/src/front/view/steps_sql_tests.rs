//! A query's SQL on one line: whitespace collapsed, and cut to the width the
//! line has with a single ellipsis where the cut landed.

use super::steps_sql::one_line;

/// Three lines of the demo's third query, fitted to the 60 columns an 80-wide
/// screen leaves the detail column.
#[test]
fn a_three_line_query_fits_sixty_columns_with_one_ellipsis() {
    let sql = "SELECT COUNT(*) AS orders_since_december,\n       MIN(order_date) AS first_order_date,\n       MAX(order_date) AS last_order_date\n  FROM orders\n WHERE order_date >= '2025-12-01'";
    let line = one_line(sql, 60);
    assert_eq!(
        line.chars().count(),
        60,
        "the line fills exactly the width it was given: {line:?}"
    );
    assert_eq!(
        line.matches('…').count(),
        1,
        "one cut is marked once: {line:?}"
    );
    assert!(line.ends_with('…'), "the cut is visible: {line:?}");
    assert!(
        !line.contains('\n'),
        "the query is one line whatever it was written as: {line:?}"
    );
    assert!(
        line.starts_with("SELECT COUNT(*) AS orders_since_december, MIN(order_date)"),
        "the line keeps the query's own words from the start: {line:?}"
    );
}

/// A query that fits is shown whole: no ellipsis, and its whitespace collapsed
/// so a multi-line statement still reads on one line.
#[test]
fn a_query_that_fits_is_shown_on_one_line() {
    assert_eq!(
        one_line("SELECT   1\n   FROM t   WHERE a = 1", 60),
        "SELECT 1 FROM t WHERE a = 1",
        "a short query is shown whole on one line"
    );
}
