//! The prepare path and native binding: parameter-free SQL keeps the prepare
//! path, parameterized SQL rewrites to `?` markers in marker order, and
//! decimals/timestamps bind as the exact validated text.

use saya_types::{BoundParam, ParamValue};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use crate::binds::BindValue;
use crate::sqlite::execute::prepare;
use crate::{ConnectorOptions, DatabaseConnector, SqliteConnector};

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
    assert_eq!(prepared, crate::prepare_sqlite_sql(sql, 10).unwrap());
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

/// SQLite is dynamically typed, so decimal and timestamp values bind as the
/// exact validated text — including leading zeros and lowercase RFC 3339.
#[test]
fn sqlite_binds_decimal_and_timestamp_as_the_validated_text() {
    let params = vec![
        bound("amount", ParamValue::Decimal("007".to_owned())),
        bound(
            "seen_at",
            ParamValue::Timestamp("2024-02-29t03:04:05z".to_owned()),
        ),
    ];
    let (_, binds) = prepare(
        "SELECT a FROM t WHERE b = :amount AND c = :seen_at",
        10,
        &params,
    )
    .unwrap();
    match &binds[0] {
        BindValue::Decimal { value, text } => {
            assert_eq!(*value, bigdecimal::BigDecimal::from(7));
            assert_eq!(text, "007");
        }
        other => panic!("expected a decimal bind, got {other:?}"),
    }
    match &binds[1] {
        BindValue::Timestamp { text, .. } => assert_eq!(text, "2024-02-29t03:04:05z"),
        other => panic!("expected a timestamp bind, got {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_supports_parameters() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("capability.db");
    let seed = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    seed.close().await;
    let connector = SqliteConnector::open(&db, true, ConnectorOptions::default())
        .await
        .unwrap();
    assert!(connector.supports_parameters());
}
