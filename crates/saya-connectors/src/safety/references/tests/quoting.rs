//! Quoted-identifier recording: per-part quoting survives extraction so a
//! folding engine can tell `Orders` from `"Orders"`.

use super::*;

#[test]
fn quoting_is_recorded_per_part() {
    // Re-audit R1: `quote_style` must survive extraction — it is the only
    // way the review can tell `Orders` (which the engine folds) from
    // `"Orders"` (which it matches exactly). Same shape as `objects`.
    assert_eq!(quoting("SELECT * FROM Orders"), vec![vec![false]]);
    assert_eq!(quoting(r#"SELECT * FROM "Orders""#), vec![vec![true]]);
    assert_eq!(
        quoting(r#"SELECT * FROM "analytics".public.orders"#),
        vec![vec![true, false, false]]
    );
    assert_eq!(
        quoting("SELECT a FROM orders o JOIN \"customers\" c ON o.cid = c.id"),
        vec![vec![false], vec![true]]
    );
}

#[test]
fn mysql_backtick_quoting_is_recorded() {
    // Any quote style the dialect writes marks the part quoted; the
    // review only ever asks "was it quoted".
    let r = sql_references("SELECT * FROM `Orders`", SqlDialect::Mysql).unwrap();
    assert_eq!(r.objects, vec![vec!["Orders"]]);
    assert_eq!(r.object_quoting, vec![vec![true]]);
}

#[test]
fn same_spelling_with_different_quoting_are_distinct_objects() {
    // Postgres folds the unquoted factor to `orders` and reads the
    // quoted one as `Orders` — two different tables — so dedup must not
    // collapse them on spelling alone.
    let r = sql_references(r#"SELECT * FROM Orders, "Orders""#, D).unwrap();
    assert_eq!(r.objects, vec![vec!["Orders"], vec!["Orders"]]);
    assert_eq!(r.object_quoting, vec![vec![false], vec![true]]);
}
