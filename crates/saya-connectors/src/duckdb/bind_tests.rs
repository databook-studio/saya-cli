//! Tests for the DuckDB native bind path: the concrete `duckdb::types::Value`
//! encodings, the decimal-precision refusal, the `?` marker rewrite, and
//! end-to-end execution through the connector with every parameter type.
//!
//! Seeding uses a raw DuckDB connection on a temp file: the connector's
//! read-only gate must keep refusing writes, so the fixture is created
//! outside it — the same approach the SQLite tests take.

use std::path::Path;

use chrono::{FixedOffset, NaiveDate, TimeZone, Utc};
use duckdb::types::{Decimal as DuckDecimal, TimeUnit, Value};
use saya_types::{BoundParam, ConnectionError, ParamType, ParamValue, QueryRequest};
use serde_json::Value as Json;
use tempfile::TempDir;

use super::native_values;
use crate::{ConnectorOptions, DatabaseConnector, DuckDbConnector};

fn bound(name: &str, value: ParamValue) -> BoundParam {
    BoundParam {
        name: name.to_owned(),
        value,
    }
}

fn epoch_days(date: NaiveDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    date.signed_duration_since(epoch).num_days() as i32
}

/// Creates the typed fixture on a fresh file database. Three rows: the typed
/// row under test, a decoy, and a row with a NULL label.
fn seed(path: &Path) {
    let connection = duckdb::Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE params_t (
                id INTEGER, label VARCHAR, amount DECIMAL(10,2),
                day DATE, seen_at TIMESTAMP, flag BOOLEAN);
             INSERT INTO params_t VALUES
                (1, 'paris', 19.99, DATE '2024-02-29', TIMESTAMP '2024-02-29 03:04:05', true),
                (2, 'london', 5.00, DATE '2023-01-15', TIMESTAMP '2023-01-15 10:00:00', false),
                (3, NULL, 5.00, DATE '2023-01-15', TIMESTAMP '2023-01-15 10:00:00', false);",
        )
        .unwrap();
}

async fn open_fixture() -> (DuckDbConnector, TempDir) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("params.duckdb");
    seed(&path);
    let connector =
        DuckDbConnector::open(path.to_str().unwrap(), true, ConnectorOptions::default())
            .await
            .unwrap();
    (connector, dir)
}

// --- The native encodings ---

#[test]
fn native_values_encode_each_parameter_type() {
    let parsed = crate::binds::parse_bind_values(&[
        ParamValue::Null(ParamType::String),
        ParamValue::String("paris".to_owned()),
        ParamValue::Integer(-5),
        ParamValue::Boolean(true),
        ParamValue::Decimal("19.99".to_owned()),
        ParamValue::Date("2024-02-29".to_owned()),
        ParamValue::Timestamp("2024-01-01T02:00:00+02:00".to_owned()),
    ])
    .unwrap();
    let values = native_values(&parsed).unwrap();
    assert_eq!(values[0], Value::Null);
    assert_eq!(values[1], Value::Text("paris".to_owned()));
    assert_eq!(values[2], Value::BigInt(-5));
    assert_eq!(values[3], Value::Boolean(true));
    // DuckDB DECIMAL(width, scale) carrying the scaled payload: 19.99 → (4, 2, 1999).
    // Text binding was probed and rejected: DuckDB's VARCHAR→DECIMAL cast rounds
    // to the compared column's scale, so `19.99 > 19.988` would compare as false.
    assert_eq!(
        values[4],
        Value::Decimal(DuckDecimal::new(4, 2, 1999).unwrap())
    );
    // Date32 counts days since the Unix epoch.
    assert_eq!(
        values[5],
        Value::Date32(epoch_days(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()))
    );
    // TIMESTAMP is microseconds since epoch at the UTC instant the offset
    // points to: 02:00+02:00 is 00:00Z.
    let micros = FixedOffset::east_opt(2 * 3600)
        .unwrap()
        .with_ymd_and_hms(2024, 1, 1, 2, 0, 0)
        .unwrap()
        .with_timezone(&Utc)
        .timestamp_micros();
    assert_eq!(values[6], Value::Timestamp(TimeUnit::Microsecond, micros));
}

#[test]
fn decimal_beyond_duckdb_precision_refuses_without_echoing_the_value() {
    for text in [
        "1e50".to_owned(),
        "1234567890123456789012345678901234567890123456789".to_owned(),
        "1e-50".to_owned(),
    ] {
        let parsed = crate::binds::parse_bind_values(&[ParamValue::Decimal(text.clone())]).unwrap();
        let error = native_values(&parsed).expect_err("an unrepresentable decimal must refuse");
        let message = error.to_string();
        assert!(
            matches!(error, ConnectionError::QueryFailed(_)),
            "{error:?}"
        );
        assert!(
            !message.contains(&text),
            "the refused value leaked into the error: {message}"
        );
    }
}

// --- End-to-end execution through the connector ---

#[tokio::test]
async fn typed_values_bind_without_interpolation() {
    let (connector, _dir) = open_fixture().await;
    let request = QueryRequest::with_params(
        "SELECT count(*) FROM params_t WHERE label = :label".to_owned(),
        10,
        vec![bound("label", ParamValue::String("' OR '1'='1".to_owned()))],
    );
    let result = connector.execute(request).await.unwrap();
    assert_eq!(result.rows, vec![Json::Array(vec![Json::from(0)])]);
}

#[tokio::test]
async fn every_parameter_type_compares_against_the_typed_fixture() {
    let (connector, _dir) = open_fixture().await;

    // A decimal param keeps its own scale: 19.99 > 19.988 is exactly true —
    // a text bind would round 19.988 to the column's DECIMAL(10,2) and
    // wrongly return zero rows. The just-above probe value must exclude it.
    for (min, expected) in [("19.988", 1), ("19.991", 0)] {
        let result = connector
            .execute(QueryRequest::with_params(
                "SELECT count(*) FROM params_t WHERE amount > :min".to_owned(),
                10,
                vec![bound("min", ParamValue::Decimal(min.to_owned()))],
            ))
            .await
            .unwrap();
        assert_eq!(result.rows[0][0], Json::from(expected), "min = {min}");
    }

    // The bound names are given in an order that differs from the SQL's
    // marker order, proving the bind list follows the markers, not the input.
    let params = vec![
        bound("n", ParamValue::Integer(1)),
        bound("day", ParamValue::Date("2024-02-29".to_owned())),
        bound("flag", ParamValue::Boolean(true)),
        bound(
            "seen_at",
            ParamValue::Timestamp("2024-02-29T03:04:05Z".to_owned()),
        ),
        bound("label", ParamValue::String("paris".to_owned())),
    ];
    let result = connector
        .execute(QueryRequest::with_params(
            "SELECT id, label, amount, day, seen_at FROM params_t \
             WHERE day = :day AND id = :n AND flag = :flag \
             AND seen_at = :seen_at AND label = :label"
                .to_owned(),
            10,
            params,
        ))
        .await
        .unwrap();
    assert_eq!(result.row_count, 1);
    assert_eq!(
        result.rows[0],
        Json::Array(vec![
            Json::from(1),
            Json::from("paris"),
            Json::from("19.99"),
            Json::from("2024-02-29"),
            Json::from("2024-02-29T03:04:05+00:00"),
        ])
    );
}

#[tokio::test]
async fn repeated_names_bind_each_occurrence() {
    let (connector, _dir) = open_fixture().await;
    let result = connector
        .execute(QueryRequest::with_params(
            "SELECT count(*) FROM params_t \
             WHERE id = :n OR (id = :n AND label = :label)"
                .to_owned(),
            10,
            vec![
                bound("n", ParamValue::Integer(1)),
                bound("label", ParamValue::String("london".to_owned())),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(result.rows[0][0], Json::from(1), "only the typed row");
}

#[tokio::test]
async fn null_binds_compare_against_nulls() {
    let (connector, _dir) = open_fixture().await;
    let null = vec![bound("label", ParamValue::Null(ParamType::String))];
    let distinct = connector
        .execute(QueryRequest::with_params(
            "SELECT count(*) FROM params_t WHERE label IS NOT DISTINCT FROM :label".to_owned(),
            10,
            null.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(distinct.rows[0][0], Json::from(1), "the NULL-label row");
    let equal = connector
        .execute(QueryRequest::with_params(
            "SELECT count(*) FROM params_t WHERE label = :label".to_owned(),
            10,
            null,
        ))
        .await
        .unwrap();
    assert_eq!(
        equal.rows[0][0],
        Json::from(0),
        "equality never matches NULL"
    );
}

#[tokio::test]
async fn parameter_free_sql_still_executes_unchanged() {
    let (connector, _dir) = open_fixture().await;
    let result = connector
        .execute(QueryRequest::new(
            "SELECT id FROM params_t ORDER BY id".to_owned(),
            2,
        ))
        .await
        .unwrap();
    assert_eq!(result.row_count, 2);
    assert!(result.truncated, "the row cap must still apply");
}

#[tokio::test]
async fn duckdb_supports_parameters() {
    let connector = DuckDbConnector::open(":memory:", false, ConnectorOptions::default())
        .await
        .unwrap();
    assert!(connector.supports_parameters());
}
