//! CTE alias visibility: a CTE name is a reference, never an object, and
//! hides a same-named base table only inside the scope that declares it.

use super::*;

#[test]
fn cte_defined_in_subquery_does_not_hide_outer_table_cross_join() {
    // A CTE is visible only inside the query that declares it. Here
    // `realtable` is declared inside the derived table; the outer
    // CROSS JOIN reads the base table of the same name and it must be
    // reported.
    let r = sql_references(
        "SELECT * FROM (WITH realtable AS (SELECT 1) SELECT * FROM realtable) sub \
         CROSS JOIN realtable",
        D,
    )
    .unwrap();
    assert_eq!(r.objects, vec![vec!["realtable"]]);
    assert!(!r.partial);
}

#[test]
fn cte_defined_in_subquery_does_not_hide_outer_table_comma_join() {
    // Same scope bug through the comma-join form.
    let r = sql_references(
        "SELECT * FROM (WITH realtable AS (SELECT 1) SELECT * FROM realtable) sub, realtable",
        D,
    )
    .unwrap();
    assert_eq!(r.objects, vec![vec!["realtable"]]);
    assert!(!r.partial);
}

#[test]
fn cte_in_nested_subquery_does_not_hide_outer_table() {
    // The CTE sits two levels down; the trailing `t` after the outer
    // derived table is the base table.
    let r = sql_references(
        "SELECT * FROM (SELECT * FROM (WITH t AS (SELECT 1) SELECT * FROM t) x) y, t",
        D,
    )
    .unwrap();
    assert_eq!(r.objects, vec![vec!["t"]]);
    assert!(!r.partial);
}

#[test]
fn plain_cte_reference_is_still_not_an_object() {
    // A CTE in the statement's own WITH hides its name for the whole
    // query: `t` is the CTE, not a base table.
    let r = sql_references("WITH t AS (SELECT 1) SELECT * FROM t", D).unwrap();
    assert_eq!(r.objects, Vec::<Vec<String>>::new());
    assert!(!r.partial);
}

#[test]
fn cte_shadows_same_named_table_only_inside_its_scope() {
    // Within the WITH's subtree `t` resolves to the CTE, so the base
    // table of the same name is absent there; a differently-named table
    // beside it is still reported.
    assert_eq!(
        objects("WITH t AS (SELECT 1) SELECT * FROM t, u"),
        vec![vec!["u"]]
    );
    // A subquery in the main body is still inside the CTE's scope.
    assert_eq!(
        objects("WITH t AS (SELECT 1) SELECT * FROM (SELECT * FROM t) s, u"),
        vec![vec!["u"]]
    );
}

#[test]
fn recursive_cte_self_reference_is_not_an_object() {
    // A recursive CTE referencing itself inside its own definition is a
    // CTE reference, not the base table.
    let r = sql_references(
        "WITH RECURSIVE t AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM t) SELECT * FROM t",
        D,
    )
    .unwrap();
    assert_eq!(r.objects, Vec::<Vec<String>>::new());
    assert!(!r.partial);
}

#[test]
fn nested_with_same_alias_at_two_levels() {
    // The same alias defined at two nested scopes: the inner `t` is the
    // inner CTE and the trailing `t` is the outer CTE; both are CTE
    // references, so nothing is reported. Leaving the inner scope must
    // not drop the outer definition.
    let r = sql_references(
        "WITH t AS (SELECT 1) SELECT * FROM (WITH t AS (SELECT 2) SELECT * FROM t) x, t",
        D,
    )
    .unwrap();
    assert_eq!(r.objects, Vec::<Vec<String>>::new());
    assert!(!r.partial);
}

#[test]
fn nonrecursive_cte_body_sees_the_base_table() {
    // A non-recursive CTE cannot see its own alias: the `orders` inside
    // the body is the base table and must be reported; the `orders` in
    // the main body is the CTE and must not be.
    let r = sql_references(
        "WITH orders AS (SELECT * FROM orders WHERE x > 1) SELECT * FROM orders",
        D,
    )
    .unwrap();
    assert_eq!(r.objects, vec![vec!["orders"]]);
    assert!(!r.partial);
}

#[test]
fn later_cte_alias_not_visible_in_earlier_body() {
    // CTE bodies see only earlier CTEs of the same WITH: `b` inside `a`'s
    // body is the base table, while the main body's `a` is the CTE.
    let r = sql_references(
        "WITH a AS (SELECT * FROM b), b AS (SELECT 1) SELECT * FROM a",
        D,
    )
    .unwrap();
    assert_eq!(r.objects, vec![vec!["b"]]);
    assert!(!r.partial);
}

#[test]
fn earlier_cte_visible_in_later_body() {
    // An earlier CTE's alias is visible in a later CTE's body.
    let r = sql_references(
        "WITH a AS (SELECT 1), b AS (SELECT * FROM a) SELECT * FROM b",
        D,
    )
    .unwrap();
    assert_eq!(r.objects, Vec::<Vec<String>>::new());
    assert!(!r.partial);
}

#[test]
fn subquery_inside_nonrecursive_cte_body_sees_the_base_table() {
    // The derived subquery sits inside the CTE's own body, which cannot
    // see its own alias: `t` there is the base table, not the CTE.
    assert_eq!(
        objects("WITH t AS (SELECT * FROM (SELECT * FROM t) s) SELECT * FROM t"),
        vec![vec!["t"]]
    );
}
