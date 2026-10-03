use std::future::poll_fn;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use saya_agent::CancellationToken;
use saya_connectors::{CancelRequestOutcome, ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use tokio::sync::{Mutex, oneshot};

static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

use super::cancellation::execute_with_cancellation;

struct ExecutionTrace {
    first_pending: Mutex<Option<oneshot::Sender<()>>>,
    settled: AtomicBool,
    cancel_calls: AtomicUsize,
    cancel_outcome: Mutex<Option<CancelRequestOutcome>>,
}

struct TracedSqlite {
    inner: SqliteConnector,
    trace: Arc<ExecutionTrace>,
}

#[async_trait]
impl DatabaseConnector for TracedSqlite {
    fn dialect(&self) -> SqlDialect {
        self.inner.dialect()
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        self.inner.connect().await
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        self.inner.schema().await
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        let mut execution = Box::pin(self.inner.execute(request));
        let mut first_pending = self.trace.first_pending.lock().await.take();
        let result = poll_fn(|cx: &mut Context<'_>| match execution.as_mut().poll(cx) {
            Poll::Ready(result) => Poll::Ready(result),
            Poll::Pending => {
                if let Some(signal) = first_pending.take() {
                    let _ = signal.send(());
                }
                Poll::Pending
            }
        })
        .await;
        self.trace.settled.store(true, Ordering::Release);
        result
    }

    fn supports_parameters(&self) -> bool {
        self.inner.supports_parameters()
    }

    async fn cancel(&self) -> Result<(), ConnectionError> {
        self.inner.cancel().await
    }

    async fn request_cancel(&self) -> Result<CancelRequestOutcome, ConnectionError> {
        self.trace.cancel_calls.fetch_add(1, Ordering::Relaxed);
        let outcome = self.inner.request_cancel().await?;
        *self.trace.cancel_outcome.lock().await = Some(outcome);
        Ok(outcome)
    }
}

#[tokio::test]
async fn active_sqlite_cancellation_requests_interrupt_and_joins_real_execution() {
    const SLOW: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt \
        WHERE x < 500000000) SELECT count(*) FROM cnt";
    let root = std::env::temp_dir().join(format!(
        "saya-active-cancel-{}-{}",
        std::process::id(),
        FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("isolated cancellation fixture directory");
    let database = root.join("active.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .expect("synthetic SQLite database creates");
    seed.close().await;

    let connector = SqliteConnector::open(
        &database,
        false,
        ConnectorOptions {
            query_timeout_seconds: 5,
            ..Default::default()
        },
    )
    .await
    .expect("real SQLite connector opens");
    connector.connect().await.expect("real SQLite connects");
    let (first_pending, started) = oneshot::channel();
    let trace = Arc::new(ExecutionTrace {
        first_pending: Mutex::new(Some(first_pending)),
        settled: AtomicBool::new(false),
        cancel_calls: AtomicUsize::new(0),
        cancel_outcome: Mutex::new(None),
    });
    let connector = TracedSqlite {
        inner: connector,
        trace: Arc::clone(&trace),
    };
    let cancellation = CancellationToken::new();
    let started_at = Instant::now();
    let operation = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            execute_with_cancellation(&connector, QueryRequest::new(SLOW, 1), &cancellation).await
        }
    });

    started.await.expect("real execute future reached Pending");
    cancellation.cancel();
    let result = operation.await.expect("operation task joins");

    assert!(matches!(result, Err(ConnectionError::Cancelled)));
    assert_eq!(trace.cancel_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        *trace.cancel_outcome.lock().await,
        Some(CancelRequestOutcome::LocalInterruptRequested)
    );
    assert!(
        trace.settled.load(Ordering::Acquire),
        "helper returns only after the real inner execute future settles"
    );
    assert!(
        started_at.elapsed() < Duration::from_secs(5),
        "cancellation settles before the configured query deadline"
    );
    std::fs::remove_dir_all(root).expect("synthetic fixture is removed");
}
