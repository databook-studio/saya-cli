use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use saya_agent::CancellationToken;
use saya_connectors::{CancelRequestOutcome, DatabaseConnector};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use tokio::sync::{Notify, oneshot};

use super::super::query_cancellation::{QueryExecution, settle_query_execution};

struct HeldConnector {
    started: Notify,
    settle: Notify,
    cancel_requested: Notify,
}

struct CountingConnector(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl DatabaseConnector for CountingConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(QueryResult::empty(request.sql))
    }
}

#[async_trait::async_trait]
impl DatabaseConnector for HeldConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        self.started.notify_one();
        self.settle.notified().await;
        Ok(QueryResult::empty(request.sql))
    }

    async fn request_cancel(&self) -> Result<CancelRequestOutcome, ConnectionError> {
        self.cancel_requested.notify_one();
        Ok(CancelRequestOutcome::LocalInterruptRequested)
    }
}

#[tokio::test]
async fn source_cancellation_requests_interrupt_and_retains_execute_until_settled() {
    let connector = Arc::new(HeldConnector {
        started: Notify::new(),
        settle: Notify::new(),
        cancel_requested: Notify::new(),
    });
    let operation_token = CancellationToken::new();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let operation = {
        let connector = Arc::clone(&connector);
        let operation_token = operation_token.clone();
        tokio::spawn(async move {
            settle_query_execution(
                connector.as_ref(),
                QueryRequest::new("select 1", 1),
                &operation_token,
                async move {
                    let _ = cancel_rx.await;
                },
                || false,
                tokio::time::Instant::now() + std::time::Duration::from_secs(60),
            )
            .await
        })
    };

    connector.started.notified().await;
    cancel_tx.send(()).unwrap();
    connector.cancel_requested.notified().await;
    tokio::task::yield_now().await;
    assert!(!operation.is_finished());

    connector.settle.notify_one();
    assert!(matches!(
        operation.await.unwrap(),
        QueryExecution::Cancelled(Some(settled)) if settled.result.is_ok()
    ));
}

#[tokio::test]
async fn timeout_requests_interrupt_and_retains_execute_until_settled() {
    let connector = Arc::new(HeldConnector {
        started: Notify::new(),
        settle: Notify::new(),
        cancel_requested: Notify::new(),
    });
    let operation_token = CancellationToken::new();
    let operation = {
        let connector = Arc::clone(&connector);
        let operation_token = operation_token.clone();
        tokio::spawn(async move {
            settle_query_execution(
                connector.as_ref(),
                QueryRequest::new("select 1", 1),
                &operation_token,
                std::future::pending(),
                || false,
                tokio::time::Instant::now() + std::time::Duration::from_millis(10),
            )
            .await
        })
    };

    connector.started.notified().await;
    connector.cancel_requested.notified().await;
    tokio::task::yield_now().await;
    assert!(!operation.is_finished());

    connector.settle.notify_one();
    assert!(matches!(
        operation.await.unwrap(),
        QueryExecution::TimedOut(Some(settled)) if settled.result.is_ok()
    ));
}

#[tokio::test]
async fn ready_source_cancellation_prevents_first_execute_poll() {
    let polls = Arc::new(AtomicUsize::new(0));
    let connector = CountingConnector(Arc::clone(&polls));
    let outcome = settle_query_execution(
        &connector,
        QueryRequest::new("select 1", 1),
        &CancellationToken::new(),
        std::future::ready(()),
        || true,
        tokio::time::Instant::now() + std::time::Duration::from_secs(60),
    )
    .await;

    assert!(matches!(outcome, QueryExecution::Cancelled(_)));
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn expired_deadline_prevents_first_execute_poll() {
    let polls = Arc::new(AtomicUsize::new(0));
    let connector = CountingConnector(Arc::clone(&polls));
    let outcome = settle_query_execution(
        &connector,
        QueryRequest::new("select 1", 1),
        &CancellationToken::new(),
        std::future::pending(),
        || false,
        tokio::time::Instant::now() - std::time::Duration::from_millis(1),
    )
    .await;

    assert!(matches!(outcome, QueryExecution::TimedOut(_)));
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}
