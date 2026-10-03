//! Tests for the SQLite execute path: cancellation, timeouts, the
//! parameter-free prepare path, and the native parameter binding.

use std::time::Duration;
use std::{future::Future, sync::Arc, task::Poll};

use saya_types::{BoundParam, ParamValue};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use super::*;
use crate::{ConnectorOptions, DatabaseConnector, SqliteConnector};

/// A query that keeps the SQLite VM busy long enough to be interrupted. The
/// recursive CTE produces far more rows than the timeout or a cancellation
/// will let it finish.
const SLOW_QUERY: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt WHERE x < 500000000) \
     SELECT count(*) FROM cnt";

/// Opens a read-only connector against a fresh temp file. The temp dir is
/// returned so the caller keeps the file path alive for the pool, which
/// opens further connections by path on demand. The file is seeded first:
/// SQLite refuses a read-only open of a path that does not exist.
async fn open(timeout_seconds: u64) -> (Arc<SqliteConnector>, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("cancel.db");
    let seed = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    seed.close().await;
    let connector = Arc::new(
        SqliteConnector::open(
            &db,
            true,
            ConnectorOptions {
                query_timeout_seconds: timeout_seconds,
                ..Default::default()
            },
        )
        .await
        .unwrap(),
    );
    (connector, dir)
}

#[tokio::test]
async fn cancelled_query_reports_cancellation_not_timeout() {
    let (connector, _dir) = open(30).await;
    let runner = connector.clone();
    let task = tokio::spawn(async move {
        runner
            .execute(QueryRequest::new(SLOW_QUERY.to_string(), 1))
            .await
    });
    // Let the statement start before cancelling it.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        connector.request_cancel().await.unwrap(),
        crate::CancelRequestOutcome::LocalInterruptRequested
    );
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("cancelled query stops within seconds")
        .unwrap();
    assert!(
        matches!(result, Err(ConnectionError::Cancelled)),
        "a cancelled query reports cancellation, not a timeout: {result:?}"
    );
}

#[tokio::test]
async fn cancellation_during_pool_acquisition_survives_and_connector_is_reusable() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("acquire-cancel.db");
    let seed = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    seed.close().await;
    let connector = Arc::new(
        SqliteConnector::open(
            &db,
            true,
            ConnectorOptions {
                query_timeout_seconds: 30,
                max_connections: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap(),
    );
    let held = connector.pool.acquire().await.unwrap();
    let mut first = Box::pin(connector.execute(QueryRequest::new(SLOW_QUERY, 1)));
    let first_poll =
        std::future::poll_fn(|context| Poll::Ready(first.as_mut().poll(context))).await;
    assert!(
        first_poll.is_pending(),
        "the first execute is pending on the held pool connection"
    );

    assert_eq!(
        connector.request_cancel().await.unwrap(),
        crate::CancelRequestOutcome::LocalInterruptRequested
    );

    let mut second = Box::pin(connector.execute(QueryRequest::new(SLOW_QUERY, 1)));
    let second_poll =
        std::future::poll_fn(|context| Poll::Ready(second.as_mut().poll(context))).await;
    assert!(
        second_poll.is_pending(),
        "the second execute joins the cancelled epoch while acquisition is pending"
    );
    drop(held);

    let (first_result, second_result) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(first, second)
    })
    .await
    .expect("both pending attempts settle after the cancellation request");
    assert!(
        matches!(first_result, Err(ConnectionError::Cancelled)),
        "the first acquired query reports cancellation: {first_result:?}"
    );
    assert!(
        matches!(second_result, Err(ConnectionError::Cancelled)),
        "the second acquired query reports cancellation: {second_result:?}"
    );

    let fresh = connector
        .execute(QueryRequest::new("SELECT 1".to_owned(), 1))
        .await
        .expect("a new query starts with fresh cancellation state");
    assert_eq!(fresh.row_count, 1);
}

#[tokio::test]
async fn idle_cancellation_reports_no_active_operation() {
    let (connector, _dir) = open(10).await;
    assert_eq!(
        connector.request_cancel().await.unwrap(),
        crate::CancelRequestOutcome::NoActiveOperation
    );
}

#[tokio::test]
async fn deadline_path_still_reports_timeout() {
    let (connector, _dir) = open(1).await;
    let result = connector
        .execute(QueryRequest::new(SLOW_QUERY.to_string(), 1))
        .await;
    let err = result.expect_err("the deadline fires before the query finishes");
    let message = err.to_string().to_lowercase();
    assert!(
        message.contains("timed out"),
        "the deadline path reports a timeout: {message}"
    );
    assert!(
        !message.contains("cancel"),
        "a timed-out query is not reported as cancelled: {message}"
    );
}

#[tokio::test]
async fn connection_works_for_the_next_query_after_cancellation() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("reuse.db");
    let seed = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE t (id INTEGER)")
        .execute(&seed)
        .await
        .unwrap();
    sqlx::query("INSERT INTO t VALUES (1), (2), (3)")
        .execute(&seed)
        .await
        .unwrap();
    seed.close().await;

    let connector = Arc::new(
        SqliteConnector::open(
            &db,
            true,
            ConnectorOptions {
                query_timeout_seconds: 30,
                ..Default::default()
            },
        )
        .await
        .unwrap(),
    );

    let runner = connector.clone();
    let task = tokio::spawn(async move {
        runner
            .execute(QueryRequest::new(SLOW_QUERY.to_string(), 1))
            .await
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    connector.cancel().await.unwrap();
    let cancelled = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(cancelled, Err(ConnectionError::Cancelled)));

    // The same connector must serve the next query — no poisoned pool entry.
    let result = connector
        .execute(QueryRequest::new(
            "SELECT id FROM t ORDER BY id".to_string(),
            10,
        ))
        .await
        .unwrap();
    assert_eq!(result.row_count, 3);
}

// --- Native parameter binding (B1c) ---

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
