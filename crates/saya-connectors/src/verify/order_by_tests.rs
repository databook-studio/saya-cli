//! Tests for the top-level `ORDER BY` detector.

use saya_types::SqlDialect;

use super::order_by::has_top_level_order_by;

/// Postgres accepts the most syntax, so it is the default dialect here.
const D: SqlDialect = SqlDialect::Postgres;

#[test]
fn top_level_order_by_is_detected() {
    assert!(has_top_level_order_by("SELECT a FROM t ORDER BY a", D));
    assert!(has_top_level_order_by(
        "SELECT a, b FROM t WHERE a > 1 ORDER BY a DESC, b",
        D,
    ));
}

#[test]
fn no_order_by_is_not_detected() {
    assert!(!has_top_level_order_by("SELECT a FROM t", D));
    assert!(!has_top_level_order_by("SELECT a FROM t WHERE a > 1", D));
}

#[test]
fn order_by_inside_subquery_is_not_top_level() {
    // The outer query has no ORDER BY; the one inside the derived table must
    // not count, so this reports false.
    assert!(!has_top_level_order_by(
        "SELECT * FROM (SELECT a FROM t ORDER BY a) sub",
        D,
    ));
}

#[test]
fn order_by_in_string_constant_does_not_count() {
    // The literal text 'order by' is a string, not a clause; there is no
    // top-level ORDER BY, so this reports false.
    assert!(!has_top_level_order_by(
        "SELECT 'order by' AS s, a FROM t",
        D,
    ));
    assert!(!has_top_level_order_by(
        "SELECT a FROM t WHERE a = 'ORDER BY x'",
        D,
    ));
}

#[test]
fn order_by_on_set_operation_is_top_level() {
    // The ORDER BY applies to the whole UNION, so it is top-level.
    assert!(has_top_level_order_by(
        "SELECT a FROM t1 UNION SELECT b FROM t2 ORDER BY a",
        D,
    ));
}

#[test]
fn non_query_statement_is_not_ordered() {
    assert!(!has_top_level_order_by("INSERT INTO t (a) VALUES (1)", D));
}

#[test]
fn unparseable_sql_reports_false() {
    assert!(!has_top_level_order_by("SELECT FROM WHERE", D));
}

#[test]
fn multiple_statements_report_false() {
    // Two statements are not a single query; a sound answer never carries two.
    assert!(!has_top_level_order_by(
        "SELECT a FROM t ORDER BY a; SELECT b FROM u",
        D,
    ));
}
