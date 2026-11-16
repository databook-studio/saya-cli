//! Tests for the MySQL execute path: the parameter-free SQL is prepared
//! exactly as before, parameterized SQL is rewritten to `?` markers with an
//! ordered bind list, and unwired behaviour stays refused. Live-server tests
//! run only when `SAYA_TEST_MYSQL_URL` names a reachable MySQL.

use std::str::FromStr;

use saya_types::{BoundParam, ParamType, ParamValue};
use sqlx::mysql::MySqlConnectOptions;

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
    assert_eq!(prepared, crate::prepare_mysql_sql(sql, 10).unwrap());
    assert!(binds.is_empty(), "no binds without parameters");
}

#[test]
fn parameterized_sql_rewrites_to_question_markers_in_order() {
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
        "SELECT a FROM t WHERE b = ? AND c = ? AND d > ? LIMIT 11"
    );
    assert!(matches!(&binds[0], BindValue::Str(text) if text == "paris"));
    assert!(matches!(&binds[1], BindValue::Str(text) if text == "paris"));
    assert!(matches!(binds[2], BindValue::Int(5)));
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

#[tokio::test]
async fn mysql_supports_parameters() {
    let options = MySqlConnectOptions::new()
        .host("db.example.test")
        .database("warehouse");
    let connector = MySqlConnector::from_options(options, "warehouse", ConnectorOptions::default());
    assert!(connector.supports_parameters());
}

/// Mirrors `localhost_mysql_fixture_round_trips_connector_contract`: typed
/// columns, one parameterized query per declared type, an injection attempt
/// that must return zero rows, and a typed null. Runs only with a reachable
/// server.
#[tokio::test]
async fn live_mysql_binds_typed_parameters_natively() {
    let Ok(url) = std::env::var("SAYA_TEST_MYSQL_URL") else {
        return;
    };
    let options = MySqlConnectOptions::from_str(&url).expect("SAYA_TEST_MYSQL_URL must be valid");
    let database = options
        .get_database()
        .expect("SAYA_TEST_MYSQL_URL must select a database")
        .to_owned();
    let table = format!("saya_param_fixture_{}", std::process::id());
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    sqlx::query(&format!(
        "CREATE TABLE `{table}` (id INT PRIMARY KEY, amount DECIMAL(4,2), flag BOOLEAN, day DATE, seen_at DATETIME, label VARCHAR(16))"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(&format!(
        "INSERT INTO `{table}` VALUES (1, 12.50, true, '2024-02-29', '2024-02-29 03:04:05', 'alpha'), (2, 3.25, false, '2024-01-01', '2024-01-01 00:00:00', 'beta')"
    ))
    .execute(&pool)
    .await
    .unwrap();

    let connector = MySqlConnector::from_options(options, &database, ConnectorOptions::default());

    for (sql, name, value, expected) in [
        (
            "SELECT id FROM {table} WHERE id = :id",
            "id",
            ParamValue::Integer(2),
            serde_json::json!([2]),
        ),
        (
            "SELECT id FROM {table} WHERE amount = :amount",
            "amount",
            ParamValue::Decimal("3.25".to_owned()),
            serde_json::json!([2]),
        ),
        (
            "SELECT id FROM {table} WHERE flag = :flag",
            "flag",
            ParamValue::Boolean(false),
            serde_json::json!([2]),
        ),
        (
            "SELECT id FROM {table} WHERE day = :day",
            "day",
            ParamValue::Date("2024-02-29".to_owned()),
            serde_json::json!([1]),
        ),
        (
            "SELECT id FROM {table} WHERE seen_at = :seen_at",
            "seen_at",
            ParamValue::Timestamp("2024-01-01T00:00:00Z".to_owned()),
            serde_json::json!([2]),
        ),
        (
            "SELECT id FROM {table} WHERE label = :label",
            "label",
            ParamValue::String("alpha".to_owned()),
            serde_json::json!([1]),
        ),
        (
            "SELECT :v IS NULL AS is_null",
            "v",
            ParamValue::Null(ParamType::String),
            serde_json::json!([1]),
        ),
    ] {
        let sql = sql.replace("{table}", &table);
        let result = connector
            .execute(QueryRequest::with_params(
                sql.clone(),
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
            format!("SELECT id FROM `{table}` WHERE label = :label"),
            10,
            vec![bound("label", ParamValue::String("' OR 1=1 --".to_owned()))],
        ))
        .await
        .unwrap();
    assert_eq!(
        result.row_count, 0,
        "an interpolated value would match every row"
    );

    sqlx::query(&format!("DROP TABLE `{table}`"))
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
