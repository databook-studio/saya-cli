//! Tests for the PostgreSQL execute path: the parameter-free SQL is prepared
//! exactly as before, parameterized SQL is rewritten to `$n` markers with an
//! ordered bind list, and unwired behaviour stays refused. Live-server tests
//! run only when `SAYA_TEST_POSTGRES_URL` names a reachable PostgreSQL.

use std::str::FromStr;

use saya_types::{BoundParam, ParamValue};
use sqlx::postgres::PgConnectOptions;

use super::*;
use crate::{ConnectorOptions, DatabaseConnector};

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
    assert_eq!(prepared, crate::prepare_postgres_sql(sql, 10).unwrap());
    assert!(binds.is_empty(), "no binds without parameters");
}

#[test]
fn parameterized_sql_rewrites_to_dollar_markers_in_order() {
    let params = vec![
        bound("city", ParamValue::String("paris".to_owned())),
        bound("floor", ParamValue::Integer(5)),
    ];
    let (prepared, binds) = prepare(
        "SELECT a FROM t WHERE b = :city AND c = :city AND d > :floor LIMIT 1000000",
        10,
        &params,
    )
    .unwrap();
    assert_eq!(
        prepared,
        "SELECT a FROM t WHERE b = $1 AND c = $1 AND d > $2 LIMIT 11"
    );
    assert!(matches!(&binds[0], BindValue::Str(text) if text == "paris"));
    assert!(matches!(binds[1], BindValue::Int(5)));
    assert_eq!(binds.len(), 2, "a repeated :name binds its value once");
}

#[test]
fn unbalanced_names_are_refused_without_values() {
    let params = vec![bound("city", ParamValue::String("paris".to_owned()))];
    let error = prepare("SELECT a FROM t WHERE b = :city AND c = :year", 10, &params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("year"), "{error}");
    assert!(!error.contains("paris"), "a value leaked into the error");
}

#[test]
fn invalid_decimal_text_is_refused_before_any_bind() {
    let params = vec![bound(
        "amount",
        ParamValue::Decimal("not-a-number".to_owned()),
    )];
    let error = prepare("SELECT a FROM t WHERE b = :amount", 10, &params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("decimal"), "{error}");
    assert!(
        !error.contains("not-a-number"),
        "a value leaked into the error"
    );
}

#[tokio::test]
async fn postgres_supports_parameters() {
    let connector =
        PostgresConnector::from_options(PgConnectOptions::new(), ConnectorOptions::default());
    assert!(connector.supports_parameters());
}

/// Mirrors `live_postgres_executes_and_marks_truncation`: typed columns, one
/// parameterized query per declared type, an injection attempt that must
/// return zero rows, and a typed null. Runs only with a reachable server.
#[tokio::test]
async fn live_postgres_binds_typed_parameters_natively() {
    let Ok(url) = std::env::var("SAYA_TEST_POSTGRES_URL") else {
        return;
    };
    let setup = sqlx::PgPool::connect(&url).await.unwrap();
    for statement in [
        "DROP TABLE IF EXISTS saya_param_fixture",
        "CREATE TABLE saya_param_fixture (id INTEGER PRIMARY KEY, amount NUMERIC, flag BOOLEAN, day DATE, seen_at TIMESTAMPTZ, label TEXT)",
        "INSERT INTO saya_param_fixture VALUES (1, 12.50, true, '2024-02-29', '2024-02-29T03:04:05Z', 'alpha'), (2, 3.25, false, '2024-01-01', '2024-01-01T00:00:00Z', 'beta')",
    ] {
        sqlx::query(statement).execute(&setup).await.unwrap();
    }

    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap();
    let connector = PostgresConnector::from_options(options, ConnectorOptions::default());

    for (sql, name, value, expected) in [
        (
            "SELECT id FROM saya_param_fixture WHERE id = :id",
            "id",
            ParamValue::Integer(2),
            serde_json::json!([2]),
        ),
        (
            "SELECT id FROM saya_param_fixture WHERE amount = :amount",
            "amount",
            ParamValue::Decimal("3.25".to_owned()),
            serde_json::json!([2]),
        ),
        (
            "SELECT id FROM saya_param_fixture WHERE flag = :flag",
            "flag",
            ParamValue::Boolean(false),
            serde_json::json!([2]),
        ),
        (
            "SELECT id FROM saya_param_fixture WHERE day = :day",
            "day",
            ParamValue::Date("2024-02-29".to_owned()),
            serde_json::json!([1]),
        ),
        (
            "SELECT id FROM saya_param_fixture WHERE seen_at = :seen_at",
            "seen_at",
            ParamValue::Timestamp("2024-01-01T00:00:00Z".to_owned()),
            serde_json::json!([2]),
        ),
        (
            "SELECT id FROM saya_param_fixture WHERE label = :label",
            "label",
            ParamValue::String("alpha".to_owned()),
            serde_json::json!([1]),
        ),
        (
            "SELECT :v IS NULL AS is_null",
            "v",
            ParamValue::Null,
            serde_json::json!([true]),
        ),
    ] {
        let result = connector
            .execute(QueryRequest::with_params(
                sql.to_string(),
                10,
                vec![bound(name, value)],
            ))
            .await
            .unwrap_or_else(|error| panic!("{sql} must bind natively: {error}"));
        assert_eq!(result.rows, vec![expected], "{sql}");
    }

    // A value carrying SQL syntax binds as a literal: zero rows, not all rows.
    let result = connector
        .execute(QueryRequest::with_params(
            "SELECT id FROM saya_param_fixture WHERE label = :label".to_string(),
            10,
            vec![bound("label", ParamValue::String("' OR 1=1 --".to_owned()))],
        ))
        .await
        .unwrap();
    assert_eq!(
        result.row_count, 0,
        "an interpolated value would match every row"
    );

    sqlx::query("DROP TABLE saya_param_fixture")
        .execute(&setup)
        .await
        .unwrap();
}
