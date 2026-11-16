use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
};
use tokio::sync::{Notify, oneshot};

use saya_agent::CancellationToken;
use saya_connectors::DatabaseConnector;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

use super::super::connect_deadline::{ConnectionSetup, ConnectionStop, connect_bounded};
use super::{DiscoveryOutcome, settle_schema_discovery};

#[tokio::test]
async fn cancelled_schema_discovery_remains_owned_until_it_returns() {
    let started = std::sync::Arc::new(Notify::new());
    let settle = std::sync::Arc::new(Notify::new());
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let (stop_observed_tx, stop_observed_rx) = oneshot::channel();
    let discovery = {
        let started = std::sync::Arc::clone(&started);
        let settle = std::sync::Arc::clone(&settle);
        async move {
            started.notify_one();
            settle.notified().await;
            "late schema"
        }
    };
    let task = tokio::spawn(settle_schema_discovery(
        discovery,
        async move {
            let _ = cancel_rx.await;
            let _ = stop_observed_tx.send(());
        },
        || false,
        tokio::time::Instant::now() + std::time::Duration::from_secs(60),
    ));

    started.notified().await;
    cancel_tx.send(()).unwrap();
    stop_observed_rx.await.unwrap();
    assert!(!task.is_finished());
    settle.notify_one();
    assert!(matches!(task.await.unwrap(), DiscoveryOutcome::Cancelled));
}

#[tokio::test]
async fn timed_out_schema_discovery_is_awaited_and_discarded() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = oneshot::channel();
    let (second_poll_tx, second_poll_rx) = oneshot::channel();
    let (settle_tx, settle_rx) = oneshot::channel();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(100);
    let discovery = {
        let polls = Arc::clone(&polls);
        let mut started_tx = Some(started_tx);
        let mut second_poll_tx = Some(second_poll_tx);
        let mut settle_rx = settle_rx;
        poll_fn(move |cx| {
            match polls.fetch_add(1, Ordering::Relaxed) {
                0 => {
                    let _ = started_tx.take().unwrap().send(());
                }
                1 => {
                    let _ = second_poll_tx.take().unwrap().send(());
                }
                _ => {}
            }
            match Pin::new(&mut settle_rx).poll(cx) {
                Poll::Ready(_) => Poll::Ready("late schema"),
                Poll::Pending => Poll::Pending,
            }
        })
    };
    let task = tokio::spawn(settle_schema_discovery(
        discovery,
        std::future::pending(),
        || false,
        deadline,
    ));

    tokio::time::timeout_at(deadline + std::time::Duration::from_secs(1), started_rx)
        .await
        .expect("schema discovery starts before the deadline")
        .unwrap();
    assert!(
        tokio::time::timeout_at(deadline, std::future::pending::<()>())
            .await
            .is_err()
    );
    tokio::time::timeout_at(deadline + std::time::Duration::from_secs(1), second_poll_rx)
        .await
        .expect("settlement keeps polling the schema future")
        .unwrap();
    assert!(!task.is_finished());
    settle_tx.send(()).unwrap();
    assert!(matches!(task.await.unwrap(), DiscoveryOutcome::TimedOut));
}

#[tokio::test]
async fn ready_source_cancellation_prevents_first_discovery_poll() {
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&polls);
    let discovery = poll_fn(move |_| {
        observed.fetch_add(1, Ordering::Relaxed);
        Poll::Ready("schema")
    });

    let outcome = settle_schema_discovery(
        discovery,
        std::future::ready(()),
        || true,
        tokio::time::Instant::now() + std::time::Duration::from_secs(60),
    )
    .await;

    assert!(matches!(outcome, DiscoveryOutcome::Cancelled));
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn expired_deadline_prevents_first_discovery_poll() {
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&polls);
    let discovery = poll_fn(move |_| {
        observed.fetch_add(1, Ordering::Relaxed);
        Poll::Ready("schema")
    });

    let outcome = settle_schema_discovery(
        discovery,
        std::future::pending(),
        || false,
        tokio::time::Instant::now() - std::time::Duration::from_millis(1),
    )
    .await;

    assert!(matches!(outcome, DiscoveryOutcome::TimedOut));
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}

/// A connector whose dial never completes, the slow-endpoint shape the
/// request bounds exist for.
struct PendingConnect;

#[async_trait::async_trait]
impl DatabaseConnector for PendingConnect {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        std::future::pending::<Result<(), ConnectionError>>().await
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(request.sql))
    }
}

/// The outer bound is the assertion: setup must return without waiting on the
/// connector's own (much longer) timeout.
async fn bounded_connect(
    source_cancelled: impl std::future::Future<Output = ()>,
    deadline: tokio::time::Instant,
) -> ConnectionSetup {
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        connect_bounded(&PendingConnect, source_cancelled, deadline),
    )
    .await
    .expect("connection setup is bounded by the request")
}

#[tokio::test]
async fn a_pending_connect_returns_the_deadline_result() {
    let outcome = bounded_connect(
        std::future::pending(),
        tokio::time::Instant::now() + std::time::Duration::from_millis(10),
    )
    .await;
    let ConnectionSetup::Stopped(stop) = outcome else {
        panic!("a pending connect is stopped at the deadline");
    };
    assert_eq!(stop, ConnectionStop::TimedOut);
    assert_eq!(
        stop.message(),
        "the database operation exceeded its 30-second cancellation-request deadline"
    );
}

#[tokio::test]
async fn a_pending_connect_returns_the_cancellation_result() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let outcome = bounded_connect(
        cancellation.cancelled(),
        tokio::time::Instant::now() + std::time::Duration::from_secs(60),
    )
    .await;
    let ConnectionSetup::Stopped(stop) = outcome else {
        panic!("a pending connect is stopped by cancellation");
    };
    assert_eq!(stop, ConnectionStop::Cancelled);
    assert_eq!(stop.message(), "the call was cancelled");
}
