//! Which object and column names are extracted across SQL shapes: name
//! part-counts, joins, subqueries, aliases, literals, and determinism.

use super::*;

#[test]
fn catalog_qualified_name_keeps_three_parts() {
    // Spec case 1: catalog.schema.table → 3 parts, plus the column.
    let r = sql_references("SELECT id FROM analytics.public.orders", D).unwrap();
    assert_eq!(r.objects, vec![vec!["analytics", "public", "orders"]]);
    assert_eq!(r.columns, vec!["id".to_string()]);
    assert!(!r.partial);
}

#[test]
fn two_part_and_one_part_names_have_no_invented_catalog() {
    // Spec case 2: only as many parts as written.
    assert_eq!(
        objects("SELECT a FROM public.orders"),
        vec![vec!["public", "orders"]]
    );
    assert_eq!(objects("SELECT a FROM orders"), vec![vec!["orders"]]);
}

#[test]
fn join_reports_both_tables_in_first_seen_order() {
    // Spec case 3.
    assert_eq!(
        objects("SELECT a FROM orders o JOIN customers c ON o.cid = c.id"),
        vec![vec!["orders"], vec!["customers"]]
    );
}

#[test]
fn subquery_table_is_reported() {
    // Spec case 4.
    assert_eq!(
        objects("SELECT x FROM (SELECT x FROM line_items) sub"),
        vec![vec!["line_items"]]
    );
}

#[test]
fn cte_name_is_not_an_object() {
    // Spec case 5: `recent` is a CTE alias; only `orders` is a real table.
    assert_eq!(
        objects("WITH recent AS (SELECT * FROM orders) SELECT * FROM recent"),
        vec![vec!["orders"]]
    );
}

#[test]
fn chained_cte_reference_excludes_inner_cte_names() {
    // A CTE referencing an earlier CTE: `b`'s body reads `a`, which is a CTE,
    // not a table. Only `orders` should survive.
    assert_eq!(
        objects("WITH a AS (SELECT * FROM orders), b AS (SELECT * FROM a) SELECT * FROM b"),
        vec![vec!["orders"]]
    );
}

#[test]
fn table_alias_is_not_an_object() {
    // Spec case 6.
    assert_eq!(objects("SELECT * FROM orders o"), vec![vec!["orders"]]);
}

#[test]
fn qualified_column_keeps_only_the_column_part() {
    // Spec case 7: `o.id` → column `id`. The qualifier `o` is an alias and
    // must never appear as a column. We keep the LAST identifier part only,
    // so `schema.table.col` would also reduce to `col` — consistent and
    // never invents a column from a schema/alias part.
    assert_eq!(columns("SELECT o.id FROM orders o"), vec!["id".to_string()]);
    assert!(
        !columns("SELECT o.id FROM orders o")
            .iter()
            .any(|c| c == "o")
    );
}

#[test]
fn unparseable_sql_returns_none() {
    // Spec case 8.
    assert!(sql_references("SELECT FROM WHERE", D).is_none());
    assert!(sql_references("not sql at all :::", D).is_none());
}

#[test]
fn case_is_preserved_exactly() {
    // Spec case 9: no folding or normalisation.
    let r = sql_references("SELECT ID FROM Orders", D).unwrap();
    assert_eq!(r.objects, vec![vec!["Orders"]]);
    assert_eq!(r.columns, vec!["ID".to_string()]);
}

#[test]
fn output_is_deterministic_for_repeated_input() {
    // Spec case 10.
    let one = sql_references("SELECT a, b FROM t JOIN s ON t.x = s.y", D).unwrap();
    let two = sql_references("SELECT a, b FROM t JOIN s ON t.x = s.y", D).unwrap();
    assert_eq!(one, two);
    // Dedup preserves first-seen order even with repeats.
    let r = sql_references("SELECT a, a, b FROM t, t", D).unwrap();
    assert_eq!(r.columns, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(r.objects, vec![vec!["t"]]);
}

#[test]
fn string_literal_does_not_leak_into_names() {
    // Spec case 12: a single-quoted string literal is data, never a name,
    // and must not ride along in any object or column. (A double-quoted
    // identifier would be a real table name and is out of scope here.)
    let r = sql_references(
        "SELECT 'SENTINELLITERAL' AS label, name FROM users WHERE token = 'SENTINELLITERAL'",
        D,
    )
    .unwrap();
    let blob = format!("{r:?}");
    assert!(
        !blob.contains("SENTINELLITERAL"),
        "literal leaked into output: {blob}"
    );
    assert_eq!(r.objects, vec![vec!["users"]]);
    // `name` and `token` are columns; the aliased literal contributes no
    // column, and the string in the WHERE stays out of the column list.
    assert_eq!(r.columns, vec!["name".to_string(), "token".to_string()]);
}

#[test]
fn set_operation_reports_all_sides() {
    assert_eq!(
        objects("SELECT a FROM t UNION SELECT a FROM u"),
        vec![vec!["t"], vec!["u"]]
    );
}

#[test]
fn output_column_alias_is_not_a_column() {
    // `SELECT x AS y FROM t` reports column `x`, not the output alias `y`.
    assert_eq!(columns("SELECT x AS y FROM t"), vec!["x".to_string()]);
    assert!(!columns("SELECT x AS y FROM t").iter().any(|c| c == "y"));
}

#[test]
fn clean_query_is_not_partial() {
    // A plain select with no unmodelled construct is fully extracted.
    assert!(!partial("SELECT id, name FROM users WHERE id = 1"));
    assert!(!partial("SELECT a FROM t JOIN s ON t.x = s.y"));
}
