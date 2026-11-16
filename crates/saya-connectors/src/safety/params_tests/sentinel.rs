use super::*;

#[test]
fn row_cap_and_guard_still_apply_with_params() {
    let v = bound("v", ParamValue::String("s".to_owned()));
    let err = prepare_with_params(
        "SELECT a FROM t WHERE b = :v",
        0,
        SqlDialect::Sqlite,
        std::slice::from_ref(&v),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("row limit"), "{err}");
    let err = prepare_with_params(
        "SELECT nextval(:s) FROM t",
        10,
        SqlDialect::Postgres,
        &[bound("s", ParamValue::String("seq".to_owned()))],
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("nextval"), "{err}");
}

#[test]
fn limit_placeholder_is_replaced_by_the_cap() {
    let err = prepare_with_params(
        "SELECT a FROM t LIMIT :rows",
        10,
        SqlDialect::Sqlite,
        &[bound("rows", ParamValue::Integer(5))],
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("rows"), "{err}");
}

#[test]
fn sentinel_prefix_in_input_is_refused_without_values() {
    let err = prepare_with_params(
        "SELECT a FROM t WHERE b = :v OR c = '__saya_bind_0'",
        10,
        SqlDialect::Sqlite,
        &[bound("v", ParamValue::String("secret-value".to_owned()))],
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("reserved marker prefix"), "{err}");
    assert!(!err.contains("secret-value"), "{err}");
}

/// A string literal whose BACKSLASH-UNESCAPED value is the reserved marker
/// prefix (MySQL, Snowflake and BigQuery unescape backslash sequences while
/// parsing) must be refused like a plain occurrence: never a panic, never a
/// corrupted value pairing, and no bound value in the message.
#[test]
fn an_unescaped_literal_that_prints_the_sentinel_prefix_is_refused() {
    let sql = r#"SELECT '\_\_saya\_bind\_' AS x, :v FROM t"#;
    assert!(
        !sql.contains("__saya_bind_"),
        "the raw text carries no prefix"
    );
    for dialect in [
        SqlDialect::Mysql,
        SqlDialect::Snowflake,
        SqlDialect::BigQuery,
    ] {
        let result = prepare_with_params(
            sql,
            10,
            dialect,
            &[bound("v", ParamValue::String("secret-value".to_owned()))],
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("reserved marker prefix"), "{dialect:?}: {err}");
        assert!(!err.contains("secret-value"), "{err}");
    }
}
