//! Unit tests for the canonical `objects` rendering (A2 decision 2, re-audit
//! R1 decision 4): a part the SQL quoted renders double-quoted — with `"`
//! doubled — so `"Orders"` and `Orders` render differently; a bare part
//! renders as written; a part that could not be read back bare (an embedded
//! dot) is quoted regardless. Parts join with ".".

use super::{canonical_objects, render_object};
use saya_types::SqlDialect;

#[test]
fn plain_names_render_bare() {
    let objects = canonical_objects("SELECT a FROM db1.public.orders", SqlDialect::Postgres);
    assert_eq!(objects, vec!["db1.public.orders".to_string()]);
}

#[test]
fn quoted_part_renders_quoted_even_when_bare_shaped() {
    // Decision 4: quoting follows the SQL, not the part's shape — `"t_1$2"`
    // was quoted, so it must not render bare (import's consistency check
    // compares this form).
    let objects = canonical_objects(r#"SELECT a FROM "t_1$2""#, SqlDialect::Postgres);
    assert_eq!(objects, vec![r#""t_1$2""#.to_string()]);
}

#[test]
fn same_spelling_different_quoting_render_differently() {
    // Decision 4: the stored objects must distinguish `Orders` (bare) from
    // `"Orders"` (quoted) — different tables in a folding engine.
    let bare = canonical_objects("SELECT a FROM Orders", SqlDialect::Postgres);
    let quoted = canonical_objects(r#"SELECT a FROM "Orders""#, SqlDialect::Postgres);
    assert_eq!(bare, vec!["Orders".to_string()]);
    assert_eq!(quoted, vec![r#""Orders""#.to_string()]);
}

#[test]
fn a_dotted_table_name_stays_one_quoted_object() {
    let objects = canonical_objects(r#"SELECT id FROM "orders.v1""#, SqlDialect::Sqlite);
    assert_eq!(objects, vec![r#""orders.v1""#.to_string()]);
}

#[test]
fn embedded_quotes_are_doubled() {
    let objects = canonical_objects(r#"SELECT a FROM "weird""name""#, SqlDialect::Postgres);
    assert_eq!(objects, vec![r#""weird""name""#.to_string()]);
}

#[test]
fn a_leading_digit_is_quoted() {
    let objects = canonical_objects(r#"SELECT a FROM "1t""#, SqlDialect::Postgres);
    assert_eq!(objects, vec![r#""1t""#.to_string()]);
}

#[test]
fn render_object_mirrors_the_parts_as_written() {
    // An unquoted part renders bare; a quoted part renders quoted even when
    // bare-shaped.
    let parts = vec!["main".to_string(), "orders.v1".to_string()];
    assert_eq!(render_object(&parts, &[false, true]), r#"main."orders.v1""#);
    let plain = vec!["Orders".to_string()];
    assert_eq!(render_object(&plain, &[true]), r#""Orders""#);
    assert_eq!(render_object(&plain, &[false]), "Orders");
}

#[test]
fn unparseable_sql_yields_no_objects() {
    assert!(canonical_objects("SELECT FROM WHERE", SqlDialect::Postgres).is_empty());
}

#[test]
fn first_seen_order_is_preserved_across_joins() {
    let objects = canonical_objects(
        "SELECT a FROM orders o JOIN customers c ON o.cid = c.id",
        SqlDialect::Postgres,
    );
    assert_eq!(objects, vec!["orders".to_string(), "customers".to_string()]);
}
