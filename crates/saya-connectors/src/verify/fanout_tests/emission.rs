//! Probe emission: which join shapes yield a probe and how the base and
//! joined row counts are built from the submitted SQL.

use crate::verify::fanout::fanout_probe;

use super::D;

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
