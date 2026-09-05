//! Tests for the fan-out probe builder.

use saya_types::SqlDialect;

use super::fanout::fanout_probe;

/// Postgres accepts the most syntax (catalog-qualified names, CTEs, SELECT
/// INTO), so it is the default dialect for these tests unless a case is
/// dialect-specific.
const D: SqlDialect = SqlDialect::Postgres;

#[test]
fn plain_two_table_join_with_sum_emits_probe() {
    let probe = fanout_probe(
        "SELECT SUM(orders.amount) FROM orders JOIN items ON orders.id = items.order_id",
        D,
    )
    .expect("a SUM over a joined base table should yield a probe");
    assert_eq!(
        probe.joined_rows,
        "SELECT COUNT(*) AS n FROM orders JOIN items ON orders.id = items.order_id"
    );
    assert_eq!(probe.base_rows, "SELECT COUNT(*) AS n FROM orders");
}

#[test]
fn table_alias_is_preserved_in_base_rows() {
    let probe = fanout_probe(
        "SELECT SUM(o.amount) FROM orders o JOIN items i ON o.id = i.order_id",
        D,
    )
    .expect("an aliased base table should yield a probe");
    assert_eq!(probe.base_rows, "SELECT COUNT(*) AS n FROM orders AS o");
    assert_eq!(
        probe.joined_rows,
        "SELECT COUNT(*) AS n FROM orders AS o JOIN items AS i ON o.id = i.order_id"
    );
}

#[test]
fn where_on_base_table_only_is_carried_into_both() {
    let probe = fanout_probe(
        "SELECT SUM(orders.amount) FROM orders JOIN items ON orders.id = items.order_id \
         WHERE orders.status = 'shipped'",
        D,
    )
    .expect("a base-only WHERE should yield a probe");
    assert_eq!(
        probe.base_rows,
        "SELECT COUNT(*) AS n FROM orders WHERE orders.status = 'shipped'"
    );
    assert_eq!(
        probe.joined_rows,
        "SELECT COUNT(*) AS n FROM orders JOIN items ON orders.id = items.order_id \
         WHERE orders.status = 'shipped'"
    );
}

#[test]
fn where_touching_joined_table_is_refused() {
    assert!(
        fanout_probe(
            "SELECT SUM(orders.amount) FROM orders JOIN items ON orders.id = items.order_id \
         WHERE items.qty > 1",
            D,
        )
        .is_none()
    );
}

#[test]
fn unqualified_where_column_is_carried() {
    let probe = fanout_probe(
        "SELECT SUM(amount) FROM orders JOIN items ON orders.id = items.order_id \
         WHERE amount > 5",
        D,
    )
    .expect("an unqualified WHERE column belongs to the base scope");
    assert!(probe.base_rows.contains("WHERE amount > 5"));
    assert!(probe.joined_rows.contains("WHERE amount > 5"));
}

#[test]
fn count_distinct_only_is_refused() {
    assert!(
        fanout_probe(
            "SELECT COUNT(DISTINCT items.product_id) FROM orders \
         JOIN items ON orders.id = items.order_id",
            D,
        )
        .is_none()
    );
}

#[test]
fn min_max_only_is_refused() {
    assert!(
        fanout_probe(
            "SELECT MAX(orders.amount), MIN(orders.fee) FROM orders \
         JOIN items ON orders.id = items.order_id",
            D,
        )
        .is_none()
    );
}

#[test]
fn no_join_is_refused() {
    assert!(fanout_probe("SELECT SUM(orders.amount) FROM orders", D).is_none());
}

#[test]
fn three_way_join_base_rows_count_only_first_table() {
    let probe = fanout_probe(
        "SELECT SUM(orders.amount) FROM orders \
         JOIN items ON orders.id = items.order_id \
         JOIN products ON items.product_id = products.id",
        D,
    )
    .expect("a three-way join should yield a probe");
    assert_eq!(probe.base_rows, "SELECT COUNT(*) AS n FROM orders");
    assert!(probe.joined_rows.contains("JOIN products"));
    assert!(
        probe
            .joined_rows
            .starts_with("SELECT COUNT(*) AS n FROM orders JOIN items")
    );
}

#[test]
fn unparseable_sql_is_none_without_panic() {
    assert!(fanout_probe("SELECT FROM WHERE", D).is_none());
    assert!(fanout_probe("not sql at all :::", D).is_none());
}

#[test]
fn union_is_refused() {
    assert!(
        fanout_probe(
            "SELECT a.x FROM a JOIN b ON a.id = b.aid UNION SELECT c.x FROM c",
            D,
        )
        .is_none()
    );
}

#[test]
fn cte_with_outer_join_is_emitted() {
    // A non-recursive CTE whose outer query joins two real tables with a
    // distortable aggregate is exactly the shape this probe looks at, so it
    // emits and carries the `WITH` clause verbatim into both statements.
    let probe = fanout_probe(
        "WITH flagged AS (SELECT order_id FROM returns) \
         SELECT SUM(orders.amount) FROM orders JOIN items ON orders.id = items.order_id \
         WHERE orders.id IN (SELECT order_id FROM flagged)",
        D,
    )
    .expect("a CTE whose outer query joins two real tables should yield a probe");
    let with = "WITH flagged AS (SELECT order_id FROM returns) ";
    assert!(
        probe.joined_rows.starts_with(with),
        "joined statement must carry the WITH clause verbatim: {}",
        probe.joined_rows
    );
    assert!(
        probe.base_rows.starts_with(with),
        "base statement must carry the WITH clause verbatim: {}",
        probe.base_rows
    );
    assert!(probe.joined_rows.contains("JOIN items"));
    assert!(!probe.base_rows.contains("JOIN items"));
    assert!(probe.base_rows.contains("FROM orders"));
}

#[test]
fn cte_as_base_table_is_emitted() {
    // A CTE referenced as the base table in the outer `FROM` is a plain table
    // reference; counting its rows with and without the outer joins is exactly
    // the comparison this probe makes.
    let probe = fanout_probe(
        "WITH prep AS (SELECT id, amount FROM orders) \
         SELECT SUM(prep.amount) FROM prep JOIN items ON prep.id = items.order_id",
        D,
    )
    .expect("a CTE as the base table should yield a probe");
    assert_eq!(
        probe.joined_rows,
        "WITH prep AS (SELECT id, amount FROM orders) \
         SELECT COUNT(*) AS n FROM prep JOIN items ON prep.id = items.order_id"
    );
    assert_eq!(
        probe.base_rows,
        "WITH prep AS (SELECT id, amount FROM orders) SELECT COUNT(*) AS n FROM prep"
    );
}

#[test]
fn multiple_ctes_carried_into_both() {
    let probe = fanout_probe(
        "WITH a AS (SELECT id, amount FROM orders), b AS (SELECT order_id, qty FROM items) \
         SELECT SUM(a.amount) FROM a JOIN b ON a.id = b.order_id",
        D,
    )
    .expect("multiple CTEs should yield a probe when the outer query joins");
    let with = "WITH a AS (SELECT id, amount FROM orders), b AS (SELECT order_id, qty FROM items) ";
    assert!(
        probe.joined_rows.starts_with(with),
        "joined statement must carry every CTE: {}",
        probe.joined_rows
    );
    assert!(
        probe.base_rows.starts_with(with),
        "base statement must carry every CTE: {}",
        probe.base_rows
    );
    assert!(probe.joined_rows.contains("JOIN b"));
    assert!(!probe.base_rows.contains("JOIN b"));
}

#[test]
fn join_only_inside_cte_is_refused() {
    // The probe inspects the outer `FROM`; a fan-out buried inside a CTE is
    // invisible to it, and an outer query with no join has nothing to compare,
    // so refusing is correct.
    assert!(
        fanout_probe(
            "WITH t AS (SELECT a.x FROM a JOIN b ON a.id = b.aid) SELECT SUM(t.x) FROM t",
            D,
        )
        .is_none()
    );
}

#[test]
fn recursive_cte_is_refused() {
    // `WITH RECURSIVE` has no stable base row count.
    assert!(
        fanout_probe(
            "WITH RECURSIVE r AS \
             (SELECT 1 AS x UNION ALL SELECT x + 1 FROM r WHERE x < 5) \
             SELECT SUM(r.x) FROM r JOIN c ON r.x = c.id",
            D,
        )
        .is_none()
    );
}

#[test]
fn cte_with_where_touching_joined_is_refused() {
    assert!(
        fanout_probe(
            "WITH prep AS (SELECT id, amount FROM orders) \
             SELECT SUM(prep.amount) FROM prep JOIN items ON prep.id = items.order_id \
             WHERE items.qty > 1",
            D,
        )
        .is_none()
    );
}

#[test]
fn subquery_in_from_is_refused() {
    assert!(
        fanout_probe(
            "SELECT SUM(s.x) FROM (SELECT x FROM a) s JOIN b ON s.id = b.aid",
            D,
        )
        .is_none()
    );
}

#[test]
fn select_into_is_refused() {
    assert!(
        fanout_probe(
            "SELECT SUM(a.x) INTO out_tbl FROM a JOIN b ON a.id = b.aid",
            D,
        )
        .is_none()
    );
}

#[test]
fn comma_cross_join_is_refused() {
    assert!(fanout_probe("SELECT SUM(a.x) FROM a, b", D).is_none());
}

#[test]
fn aggregate_inside_scalar_subquery_does_not_trigger_probe() {
    // The outer query joins but has no aggregate of its own; the SUM belongs
    // to the subquery's scope and is not distorted by the outer join, so the
    // probe must not fire on it.
    assert!(
        fanout_probe(
            "SELECT (SELECT SUM(x) FROM b) FROM a JOIN c ON a.id = c.aid",
            D,
        )
        .is_none()
    );
}

#[test]
fn schema_qualified_aggregate_is_not_treated_as_builtin() {
    // A schema-qualified name is a custom function, not the built-in SUM, so it
    // is not assumed to be distorted by fan-out. Refusing is the safe answer.
    assert!(
        fanout_probe(
            "SELECT analytics.SUM(o.amount) FROM orders o \
             JOIN items i ON o.id = i.order_id",
            D,
        )
        .is_none()
    );
}

#[test]
fn emitted_statements_pass_read_only_safety_layer() {
    let probe = fanout_probe(
        "SELECT SUM(o.amount) FROM orders o JOIN items i ON o.id = i.order_id \
         WHERE o.status = 'shipped'",
        D,
    )
    .expect("probe for a base-only WHERE should be emitted");
    assert!(crate::prepare_postgres_sql(&probe.joined_rows, 1000).is_ok());
    assert!(crate::prepare_postgres_sql(&probe.base_rows, 1000).is_ok());

    // A `WITH` clause is new surface reaching the parser; the safety layer must
    // still accept both emitted statements.
    let cte_probe = fanout_probe(
        "WITH prep AS (SELECT id, amount FROM orders) \
         SELECT SUM(prep.amount) FROM prep JOIN items ON prep.id = items.order_id",
        D,
    )
    .expect("a CTE-based probe should be emitted");
    assert!(crate::prepare_postgres_sql(&cte_probe.joined_rows, 1000).is_ok());
    assert!(crate::prepare_postgres_sql(&cte_probe.base_rows, 1000).is_ok());
}
