//! Cancellation semantics: an in-flight query reports cancellation rather
//! than a timeout, an idle connector reports no active operation, and a
//! request issued while pool acquisition is pending still settles cleanly.

use std::future::Future;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use saya_types::{ConnectionError, QueryRequest};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use crate::{ConnectorOptions, DatabaseConnector, SqliteConnector};

use super::{SLOW_QUERY, open};

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
