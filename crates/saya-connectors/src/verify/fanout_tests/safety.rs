//! The emitted statements pass the read-only safety layer unchanged —
//! including the `WITH`-prefixed forms, which are new parser surface.

use crate::verify::fanout::fanout_probe;

use super::D;

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
