//! Tests for the DuckDB execute prepare path: parameter-free SQL keeps the
//! existing prepare pipeline, and parameterized SQL is rewritten to `?`
//! markers with an ordered bind list — values never touch the text.

use saya_types::{BoundParam, ParamValue};

use super::prepare;

fn bound(name: &str, value: ParamValue) -> BoundParam {
    BoundParam {
        name: name.to_owned(),
        value,
    }
}

#[test]
fn parameter_free_sql_keeps_the_prepare_path() {
    let sql = "SELECT a FROM t WHERE b = 1 LIMIT 1000000";
    let (prepared, binds) = prepare(sql, 10, &[]).unwrap();
    assert_eq!(prepared, crate::prepare_duckdb_sql(sql, 10).unwrap());
    assert!(binds.is_empty(), "no binds without parameters");
}

#[test]
fn parameterized_sql_rewrites_to_question_markers_in_order() {
    // The bound names are given in an order that differs from the SQL's
    // marker order, and `:city` repeats: the rewrite must follow the
    // markers, not the input.
    let params = vec![
        bound("floor", ParamValue::Integer(5)),
        bound("city", ParamValue::String("paris".to_owned())),
    ];
    let (prepared, binds) = prepare(
        "SELECT a FROM t WHERE b = :city AND c = :city AND d > :floor LIMIT 1000000",
        10,
        &params,
    )
    .unwrap();
    assert_eq!(
        prepared,
        "SELECT a FROM t WHERE b = ? AND c = ? AND d > ? LIMIT 11"
    );
    assert!(
        matches!(&binds[0], duckdb::types::Value::Text(text) if text == "paris"),
        "{:?}",
        binds[0]
    );
    assert!(
        matches!(&binds[1], duckdb::types::Value::Text(text) if text == "paris"),
        "{:?}",
        binds[1]
    );
    assert!(
        matches!(binds[2], duckdb::types::Value::BigInt(5)),
        "{:?}",
        binds[2]
    );
}
