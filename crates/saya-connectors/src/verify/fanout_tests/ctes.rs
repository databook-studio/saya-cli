//! CTE handling: the `WITH` clause is carried verbatim into both emitted
//! statements when the outer query joins, and CTE-shaped refusals stay
//! refused.

use crate::verify::fanout::fanout_probe;

use super::D;

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
