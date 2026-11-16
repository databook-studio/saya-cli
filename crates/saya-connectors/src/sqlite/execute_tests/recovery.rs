//! Settlement and reuse: the deadline path reports a timeout (never a
//! cancellation), and the same connector serves the next query after a
//! cancellation — no poisoned pool entry.

use std::sync::Arc;
use std::time::Duration;

use saya_types::{ConnectionError, QueryRequest};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use crate::{ConnectorOptions, DatabaseConnector, SqliteConnector};

use super::{SLOW_QUERY, open};

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
