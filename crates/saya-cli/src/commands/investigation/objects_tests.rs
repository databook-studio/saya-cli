//! Unit tests for the canonical `objects` rendering (A2 decision 2): a part
//! matching `[A-Za-z_][A-Za-z0-9_$]*` stays bare, anything else — including a
//! part with an embedded dot — is double-quoted with `"` doubled, and parts
//! join with ".".

use super::{canonical_objects, render_object};
use saya_types::SqlDialect;

#[test]
fn plain_names_render_bare() {
    let objects = canonical_objects("SELECT a FROM db1.public.orders", SqlDialect::Postgres);
    assert_eq!(objects, vec!["db1.public.orders".to_string()]);
}

#[test]
fn dollar_digits_and_underscore_stay_bare() {
    // Quoted on the way in (sqlparser may not accept an unquoted `$`), but
    // the part matches the bare class, so it renders bare.
    let objects = canonical_objects(r#"SELECT a FROM "t_1$2""#, SqlDialect::Postgres);
    assert_eq!(objects, vec!["t_1$2".to_string()]);
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
    let parts = vec!["main".to_string(), "orders.v1".to_string()];
    assert_eq!(render_object(&parts), r#"main."orders.v1""#);
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
