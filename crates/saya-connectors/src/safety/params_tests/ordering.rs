use super::*;

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
