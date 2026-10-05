//! Tests for parameter-aware preparation (`params.rs`): a query's
//! placeholders are proven to be exactly the bound names and rewritten in
//! the AST to the dialect's native markers — never substituted into text.

use saya_types::{BoundParam, ParamValue, SqlDialect};

use super::params::{prepare_with_params, sql_placeholders};

fn bound(name: &str, value: ParamValue) -> BoundParam {
    BoundParam {
        name: name.to_owned(),
        value,
    }
}

fn city() -> BoundParam {
    bound("city", ParamValue::String("paris".to_owned()))
}

fn floor() -> BoundParam {
    bound("floor", ParamValue::Integer(5))
}

fn dashed(query: &str) -> String {
    query.replace(' ', "").to_ascii_lowercase()
}

#[test]
fn placeholder_in_string_or_comment_is_not_bound() {
    let sql = "SELECT ':region' AS lit, 1 /* :floor */ FROM t";
    let prepared = prepare_with_params(sql, 10, SqlDialect::Sqlite, &[]).unwrap();
    assert!(prepared.sql.contains("':region'"), "{}", prepared.sql);
    assert!(prepared.values.is_empty());

    // A comment placeholder is not a node either: binding a name whose
    // `:name` appears only in a literal or comment is an unused extra.
    let err = prepare_with_params(sql, 10, SqlDialect::Sqlite, &[city()])
        .unwrap_err()
        .to_string();
    assert!(err.contains("city"), "{err}");
    let err = prepare_with_params(sql, 10, SqlDialect::Sqlite, &[floor()])
        .unwrap_err()
        .to_string();
    assert!(err.contains("floor"), "{err}");
    assert!(prepared.sql.contains("':region'"));
}

#[test]
fn parameter_cannot_change_query_shape() {
    let value = ParamValue::String("'1; DROP TABLE t".to_owned());
    let prepared = prepare_with_params(
        "SELECT name FROM t WHERE id = :v",
        10,
        SqlDialect::Sqlite,
        &[bound("v", value)],
    )
    .unwrap();
    assert_eq!(prepared.sql, "SELECT name FROM t WHERE id = ? LIMIT 11");
    assert_eq!(
        prepared.values,
        vec![ParamValue::String("'1; DROP TABLE t".to_owned())]
    );
    assert!(!prepared.sql.to_ascii_lowercase().contains("drop"));
}

#[test]
fn bounds_rewrite_preserves_bind_order() {
    let prepared = prepare_with_params(
        "SELECT a FROM t WHERE b = :city AND c > :floor AND d = :city LIMIT 1000000",
        10,
        SqlDialect::Sqlite,
        &[city(), floor()],
    )
    .unwrap();
    assert_eq!(
        prepared.sql,
        "SELECT a FROM t WHERE b = ? AND c > ? AND d = ? LIMIT 11"
    );
    assert_eq!(
        prepared.values,
        vec![
            ParamValue::String("paris".to_owned()),
            ParamValue::Integer(5),
            ParamValue::String("paris".to_owned()),
        ]
    );
}

#[test]
fn postgres_numbers_placeholders_by_first_occurrence() {
    let prepared = prepare_with_params(
        "SELECT a FROM t WHERE b = :city AND c > :floor AND d = :city LIMIT 1000000",
        10,
        SqlDialect::Postgres,
        &[city(), floor()],
    )
    .unwrap();
    assert_eq!(
        prepared.sql,
        "SELECT a FROM t WHERE b = $1 AND c > $2 AND d = $1 LIMIT 11"
    );
    assert_eq!(
        prepared.values,
        vec![
            ParamValue::String("paris".to_owned()),
            ParamValue::Integer(5)
        ]
    );
}

#[test]
fn question_marker_dialects_repeat_values() {
    for dialect in [
        SqlDialect::Mysql,
        SqlDialect::Sqlite,
        SqlDialect::DuckDb,
        SqlDialect::Snowflake,
        SqlDialect::BigQuery,
    ] {
        let prepared = prepare_with_params(
            "SELECT a FROM t WHERE b = :city AND c = :city LIMIT 1000000",
            10,
            dialect,
            &[city()],
        )
        .unwrap();
        assert!(
            dashed(&prepared.sql).contains("b=?andc=?"),
            "{dialect:?}: {}",
            prepared.sql
        );
        assert_eq!(
            prepared.values,
            vec![
                ParamValue::String("paris".to_owned()),
                ParamValue::String("paris".to_owned()),
            ],
            "{dialect:?}"
        );
    }
}

#[test]
fn missing_and_extra_names_are_reported_without_values() {
    let sql = "SELECT a FROM t WHERE b = :region AND c = :year";
    let secret = ParamValue::String("secret-value".to_owned());

    let err = prepare_with_params(
        sql,
        10,
        SqlDialect::Postgres,
        &[bound("region", secret.clone())],
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("year"), "{err}");
    assert!(!err.contains("secret-value"), "{err}");

    let err = prepare_with_params(
        sql,
        10,
        SqlDialect::Postgres,
        &[
            bound("region", secret.clone()),
            bound("year", ParamValue::Integer(2024)),
            bound("extra", ParamValue::String("nope".to_owned())),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("extra"), "{err}");
    assert!(!err.contains("nope"), "{err}");
}

#[test]
fn clickhouse_refuses_parameters() {
    let err = prepare_with_params(
        "SELECT a FROM t WHERE b = :city",
        10,
        SqlDialect::ClickHouse,
        &[city()],
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("parameters are not supported for ClickHouse"),
        "{err}"
    );
}

#[test]
fn write_statements_are_refused_with_params() {
    let v = bound("v", ParamValue::Integer(1));
    let err = prepare_with_params(
        "DELETE FROM t WHERE id = :v",
        10,
        SqlDialect::Sqlite,
        std::slice::from_ref(&v),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("modifies data"), "{err}");
    let err = prepare_with_params("INSERT INTO t VALUES (:v)", 10, SqlDialect::Sqlite, &[v])
        .unwrap_err()
        .to_string();
    assert!(err.contains("modifies data"), "{err}");
}

#[test]
fn sql_placeholders_are_distinct_and_in_ast_order() {
    let names = sql_placeholders(
        "SELECT a FROM t WHERE b = :alpha AND c = :beta AND d = :alpha",
        SqlDialect::Postgres,
    )
    .unwrap();
    assert_eq!(names, vec!["alpha".to_owned(), "beta".to_owned()]);
    assert!(
        sql_placeholders("SELECT 1 FROM t", SqlDialect::Postgres)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn non_name_placeholder_forms_are_refused() {
    for (sql, dialect, raw) in [
        ("SELECT a FROM t WHERE b = $1", SqlDialect::Postgres, "$1"),
        ("SELECT a FROM t WHERE b = ?", SqlDialect::Mysql, "?"),
        ("SELECT a FROM t WHERE b = ?1", SqlDialect::Sqlite, "?1"),
        ("SELECT a FROM t WHERE b = @x", SqlDialect::Sqlite, "@x"),
        ("SELECT a FROM t WHERE b = :5", SqlDialect::Sqlite, ":5"),
    ] {
        let err = sql_placeholders(sql, dialect).unwrap_err().to_string();
        assert!(err.contains(raw), "{raw}: {err}");
        assert!(err.contains(":name placeholders"), "{raw}: {err}");
    }
}

#[test]
fn parameterless_preparation_matches_prepare() {
    let sql = "SELECT a FROM t WHERE b = 1 LIMIT 1000000";
    let parameterized = prepare_with_params(sql, 10, SqlDialect::Sqlite, &[]).unwrap();
    assert_eq!(
        parameterized.sql,
        super::prepare_sqlite_sql(sql, 10).unwrap()
    );
    assert!(parameterized.values.is_empty());
}

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
fn question_markers_follow_case_text_order() {
    let sql = "SELECT CASE WHEN a = :x THEN :y WHEN a = :z THEN :w END FROM t";
    let prepared = prepare_with_params(
        sql,
        10,
        SqlDialect::Sqlite,
        &[
            bound("x", ParamValue::Integer(1)),
            bound("y", ParamValue::Integer(2)),
            bound("z", ParamValue::Integer(3)),
            bound("w", ParamValue::Integer(4)),
        ],
    )
    .unwrap();
    assert_eq!(
        prepared.values,
        vec![
            ParamValue::Integer(1),
            ParamValue::Integer(2),
            ParamValue::Integer(3),
            ParamValue::Integer(4),
        ]
    );
    assert_eq!(prepared.sql.matches('?').count(), 4, "{}", prepared.sql);
}

#[test]
fn question_markers_follow_trim_text_order() {
    let prepared = prepare_with_params(
        "SELECT TRIM(:c FROM :v) FROM t",
        10,
        SqlDialect::Mysql,
        &[
            bound("c", ParamValue::String("c-first".to_owned())),
            bound("v", ParamValue::String("v-second".to_owned())),
        ],
    )
    .unwrap();
    assert_eq!(
        prepared.values,
        vec![
            ParamValue::String("c-first".to_owned()),
            ParamValue::String("v-second".to_owned()),
        ]
    );
    assert!(
        dashed(&prepared.sql).contains("trim(?from?)"),
        "{}",
        prepared.sql
    );
}

#[test]
fn repeated_name_in_case_repeats_at_both_text_positions() {
    let prepared = prepare_with_params(
        "SELECT CASE WHEN a = :a THEN :b WHEN a = :a THEN :b END FROM t",
        10,
        SqlDialect::Sqlite,
        &[
            bound("a", ParamValue::Integer(1)),
            bound("b", ParamValue::Integer(2)),
        ],
    )
    .unwrap();
    assert_eq!(prepared.sql.matches('?').count(), 4, "{}", prepared.sql);
    assert_eq!(
        prepared.values,
        vec![
            ParamValue::Integer(1),
            ParamValue::Integer(2),
            ParamValue::Integer(1),
            ParamValue::Integer(2),
        ]
    );
}

#[test]
fn dollar_output_is_unchanged_for_case() {
    let prepared = prepare_with_params(
        "SELECT CASE WHEN a = :x THEN :y WHEN a = :z THEN :w END FROM t",
        10,
        SqlDialect::Postgres,
        &[
            bound("x", ParamValue::Integer(1)),
            bound("y", ParamValue::Integer(2)),
            bound("z", ParamValue::Integer(3)),
            bound("w", ParamValue::Integer(4)),
        ],
    )
    .unwrap();
    assert_eq!(
        prepared.sql,
        "SELECT CASE WHEN a = $1 THEN $3 WHEN a = $2 THEN $4 END FROM t LIMIT 11"
    );
    assert_eq!(
        prepared.values,
        vec![
            ParamValue::Integer(1),
            ParamValue::Integer(3),
            ParamValue::Integer(2),
            ParamValue::Integer(4),
        ]
    );
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
