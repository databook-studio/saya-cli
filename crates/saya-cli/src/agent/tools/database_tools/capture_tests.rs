//! Tests for the agent query capture hook (C1): one successful
//! single-connection `bounded_sql_query` hands the TUI the typed result the
//! model saw; fan-out and the probe tools never capture; a failure never
//! captures; an over-budget result refuses whole — never a partial.

use super::*;
use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, LocalStateEffect, ToolCall, ToolConcurrency, ToolDefinition, ToolEffect,
    ToolExecutor, run_agent_with_sink, shape_tool_result,
};
use saya_connectors::{ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use std::future::poll_fn;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use tokio::sync::{Mutex as AsyncMutex, oneshot};

use crate::{
    agent::tools::RunTools,
    connection::{ConnectionEntry, ConnectionRegistry},
};

/// The events one hook observed, shared with the test through a mutex.
type Observed = Arc<Mutex<Vec<CaptureEvent>>>;
type AgentEvents = Arc<Mutex<Vec<saya_agent::AgentEvent>>>;

static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

fn observing_hook() -> (CaptureHook, Observed) {
    let observed: Observed = Arc::new(Mutex::new(Vec::new()));
    let sink = observed.clone();
    let hook: CaptureHook = Arc::new(move |event| sink.lock().unwrap().push(event));
    (hook, observed)
}

struct RecordingSink(AgentEvents);

#[async_trait]
impl saya_agent::AgentEventSink for RecordingSink {
    async fn emit(&self, event: saya_agent::AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct ExecutionTrace {
    first_pending: AsyncMutex<Option<oneshot::Sender<()>>>,
    settled: AtomicBool,
    execute_calls: AtomicUsize,
    cancel_calls: AtomicUsize,
    cancel_outcome: AsyncMutex<Option<saya_connectors::CancelRequestOutcome>>,
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

    async fn request_cancel(
        &self,
    ) -> Result<saya_connectors::CancelRequestOutcome, ConnectionError> {
        self.trace.cancel_calls.fetch_add(1, Ordering::Relaxed);
        let outcome = self.inner.request_cancel().await?;
        *self.trace.cancel_outcome.lock().await = Some(outcome);
        Ok(outcome)
    }
}

/// A scripted connector whose result is fully controllable, so a test can
/// plant a known row count (the `observations_tests` pattern).
struct ScriptedConnector {
    rows: Vec<serde_json::Value>,
    row_count: usize,
    truncated: bool,
}

#[async_trait]
impl DatabaseConnector for ScriptedConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult {
            columns: vec!["c".into()],
            rows: self.rows.clone(),
            row_count: self.row_count,
            truncated: self.truncated,
            executed_sql: req.sql,
        })
    }
}

/// A connector whose queries always fail.
struct FailingConnector;

#[async_trait]
impl DatabaseConnector for FailingConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed("syntax error near SENTINEL"))
    }
}

/// A connector returning one cell just over the capture budget, so the
/// refusal path can be exercised end to end without serializing twice.
struct BigCellConnector;

#[async_trait]
impl DatabaseConnector for BigCellConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult {
            columns: vec!["big".into()],
            rows: vec![serde_json::json!(["x".repeat(
                crate::interactive::tui::capture::CAPTURE_BUDGET_BYTES + 1
            )])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A seeded SQLite file plus a read-only connector over it, and the temp dir
/// holding the file (kept alive for the pool). Mirrors the connector crate's
/// own test fixture: seed with a plain pool, close it, open read-only.
async fn sqlite_fixture() -> (Box<dyn DatabaseConnector>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "saya-capture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let db = dir.join("capture.db");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER, name TEXT)")
        .execute(&seed)
        .await
        .unwrap();
    let values = (0..60)
        .map(|id| format!("({id}, 'user {id}')"))
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(&format!("INSERT INTO users VALUES {values}"))
        .execute(&seed)
        .await
        .unwrap();
    seed.close().await;
    let connector = SqliteConnector::open(&db, true, ConnectorOptions::default())
        .await
        .unwrap();
    (Box::new(connector), dir)
}

fn scripted_entry() -> ConnectionEntry {
    ConnectionEntry {
        connector: Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"]), serde_json::json!(["y"])],
            row_count: 2,
            truncated: false,
        }),
        dialect: SqlDialect::Sqlite,
        profile_id: Some("p-capture-identity-sentinel".to_string()),
    }
}

fn loop_budget() -> usize {
    saya_agent::AgentLimits::default().context_byte_budget
}

#[path = "capture_tests/budget_refusal.rs"]
mod budget_refusal;
#[path = "capture_tests/model_view.rs"]
mod model_view;
#[path = "capture_tests/query_capture.rs"]
mod query_capture;
#[path = "capture_tests/serial_share.rs"]
mod serial_share;
#[path = "capture_tests/sqlite_settlement.rs"]
mod sqlite_settlement;
