//! The traced-source harness for the live candidate-vote proofs: a
//! delegate-only `TracedSqlite` (the pattern already proven twice, in
//! `interactive/sql_operation_cancellation_tests.rs` and
//! `agent/tools/database_tools/capture_tests.rs`) around a real
//! `SqliteConnector`. It fabricates nothing — every call delegates to the real
//! connector, the trace only observes: the first real `Pending` of the inner
//! execute, whether that execute settled, how often the delegated
//! `request_cancel` ran, how many queries started, and what the interrupt
//! answered.

use std::future::poll_fn;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::task::{Context, Poll};

use async_trait::async_trait;
use saya_connectors::{CancelRequestOutcome, DatabaseConnector, SqliteConnector};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use tokio::sync::{Mutex, oneshot};

pub(super) static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

/// The canonical slow source: a recursive count the connector cannot finish
/// before the question's elapsed ceiling fires (same shape as
/// `sql_operation_cancellation_tests.rs`).
pub(super) const SLOW: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt \
        WHERE x < 500000000) SELECT count(*) FROM cnt";

pub(super) struct ExecutionTrace {
    pub(super) first_pending: Mutex<Option<oneshot::Sender<()>>>,
    pub(super) settled: AtomicBool,
    pub(super) cancel_calls: AtomicUsize,
    pub(super) execute_calls: AtomicUsize,
    pub(super) cancel_outcome: Mutex<Option<CancelRequestOutcome>>,
}

pub(super) struct TracedSqlite {
    pub(super) inner: SqliteConnector,
    pub(super) trace: Arc<ExecutionTrace>,
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
        self.trace.execute_calls.fetch_add(1, Ordering::Relaxed);
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
