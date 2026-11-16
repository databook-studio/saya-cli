use super::*;

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
fn parameterless_preparation_matches_prepare() {
    let sql = "SELECT a FROM t WHERE b = 1 LIMIT 1000000";
    let parameterized = prepare_with_params(sql, 10, SqlDialect::Sqlite, &[]).unwrap();
    assert_eq!(
        parameterized.sql,
        super::prepare_sqlite_sql(sql, 10).unwrap()
    );
    assert!(parameterized.values.is_empty());
}
