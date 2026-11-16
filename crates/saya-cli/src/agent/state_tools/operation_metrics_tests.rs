//! DatabaseWait and DatabaseExecute phase samples recorded by the operation
//! owner, observed through the real tool dispatch with a production SQLite
//! connector and a lease-holding connector for the contended paths.

use crate::agent::tools::DatabaseTools;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use crate::local_metrics::{LocalMetrics, LocalPhaseSnapshot, outcome_index, phase_index};
use async_trait::async_trait;
use saya_agent::{
    AgentMetricOutcome, AgentMetricPhase, CancellationToken, ToolError, ToolExecutionContext,
    ToolExecutor,
};
use saya_connectors::{CancelRequestOutcome, ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use serde_json::json;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use std::future::poll_fn;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::Poll;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

struct LeaseConnector {
    started: Arc<Notify>,
    release: Arc<Notify>,
    executions: Arc<AtomicUsize>,
    cancel_requests: Arc<AtomicUsize>,
}

#[async_trait]
impl DatabaseConnector for LeaseConnector {
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
        self.executions.fetch_add(1, Ordering::SeqCst);
        self.started.notify_waiters();
        self.release.notified().await;
        Ok(QueryResult::empty(request.sql))
    }

    async fn request_cancel(&self) -> Result<CancelRequestOutcome, ConnectionError> {
        self.cancel_requests.fetch_add(1, Ordering::SeqCst);
        Ok(CancelRequestOutcome::LocalInterruptRequested)
    }
}

struct FailingConnector;

#[async_trait]
impl DatabaseConnector for FailingConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed("syntax error near FROM"))
    }
}

/// A connector whose live metadata operation always fails; its execute still
/// succeeds so the lease-holding execute tests are unaffected.
struct FailingSchemaConnector;

#[async_trait]
impl DatabaseConnector for FailingSchemaConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::query_failed("metadata query failed"))
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(request.sql))
    }
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-operation-metrics-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    root
}

fn samples_in(snapshot: &LocalPhaseSnapshot, phase: AgentMetricPhase) -> u64 {
    snapshot.phases[phase_index(phase).expect("known phase")].samples
}

fn outcome_in(
    snapshot: &LocalPhaseSnapshot,
    phase: AgentMetricPhase,
    outcome: AgentMetricOutcome,
) -> u64 {
    let cell = &snapshot.phases[phase_index(phase).expect("known phase")];
    cell.outcomes[outcome_index(outcome).expect("known outcome")]
}

async fn sqlite_tools(root: &Path, metrics: Option<Arc<LocalMetrics>>) -> DatabaseTools {
    let database = root.join("operation-metrics.sqlite3");
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .expect("seed SQLite metrics database");
    sqlx::query("CREATE TABLE orders (id INTEGER)")
        .execute(&pool)
        .await
        .expect("create orders table");
    sqlx::query("INSERT INTO orders (id) VALUES (1)")
        .execute(&pool)
        .await
        .expect("seed one row");
    pool.close().await;
    let connector = SqliteConnector::open(&database, true, ConnectorOptions::default())
        .await
        .expect("open production SQLite connector");
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(connector),
            dialect: SqlDialect::Sqlite,
            profile_id: None,
        },
    );
    DatabaseTools::with_registry(registry, 100, true, None).with_metrics(metrics)
}

fn lease_tools(
    started: Arc<Notify>,
    release: Arc<Notify>,
    executions: Arc<AtomicUsize>,
    cancel_requests: Arc<AtomicUsize>,
    metrics: Option<Arc<LocalMetrics>>,
) -> Arc<DatabaseTools> {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(LeaseConnector {
                started,
                release,
                executions,
                cancel_requests,
            }),
            dialect: SqlDialect::Sqlite,
            profile_id: None,
        },
    );
    Arc::new(DatabaseTools::with_registry(registry, 100, true, None).with_metrics(metrics))
}

include!("operation_metrics_tests/schema_tools.rs");
include!("operation_metrics_tests/lease_wait.rs");
include!("operation_metrics_tests/query_wait.rs");

include!("operation_metrics_tests/query_outcomes.rs");

include!("operation_metrics_tests/schema_wait.rs");

include!("operation_metrics_tests/schema_failures.rs");
