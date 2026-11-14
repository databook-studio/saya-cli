//! Refusals: query shapes the probe must not fire on — joined-table filters,
//! aggregates it cannot attribute, no join, unparsable SQL, unions,
//! subqueries, `SELECT INTO`, comma joins, and schema-qualified functions.

use crate::verify::fanout::fanout_probe;

use super::D;

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
