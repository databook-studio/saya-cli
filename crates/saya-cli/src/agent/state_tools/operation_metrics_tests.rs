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

fn tool_context(cancellation: CancellationToken) -> ToolExecutionContext {
    ToolExecutionContext {
        result_cap: 1024,
        cancellation,
    }
}

/// Drives the joined operation until it is observed pending — it owns no
/// settled result yet, so any later samples must come from a later phase.
async fn assert_pending(
    waiter: &mut tokio::task::JoinHandle<Result<serde_json::Value, ToolError>>,
    label: &str,
) {
    poll_fn(|cx| match Pin::new(&mut *waiter).poll(cx) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(result) => panic!("{label}: operation settled before the lease: {result:?}"),
    })
    .await;
}

async fn wait_for_executions(executions: &AtomicUsize, expected: usize) {
    for _ in 0..2000 {
        if executions.load(Ordering::SeqCst) == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("operation never reached {expected} connector executions");
}

/// (a) An uncontended real SQLite query through the operation owner records
/// exactly one DatabaseWait and one DatabaseExecute sample, both succeeded.
#[tokio::test]
async fn uncontended_sqlite_query_records_one_wait_and_one_execute_sample() {
    let root = temp_root("uncontended");
    let metrics = Arc::new(LocalMetrics::new());
    let tools = sqlite_tools(&root, Some(Arc::clone(&metrics))).await;
    let result = tools
        .execute_with_context(
            "bounded_sql_query",
            json!({"sql": "SELECT id FROM orders"}),
            tool_context(CancellationToken::new()),
        )
        .await
        .expect("uncontended query succeeds");
    assert!(
        result.is_object(),
        "the model received a query result: {result:?}"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::DatabaseWait),
        1,
        "one wait sample: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the uncontended acquire succeeded: {snapshot:?}"
    );
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::DatabaseExecute),
        1,
        "one execute sample: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the settled query succeeded: {snapshot:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// (b) A waiter held at the entry lease records its DatabaseWait sample only
/// once the holder releases, and its DatabaseExecute sample only once its own
/// source settles — the two intervals never cover each other.
#[tokio::test]
async fn contended_waiter_records_wait_only_after_the_lease_is_granted() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let executions = Arc::new(AtomicUsize::new(0));
    let cancel_requests = Arc::new(AtomicUsize::new(0));
    let metrics = Arc::new(LocalMetrics::new());
    let tools = lease_tools(
        Arc::clone(&started),
        Arc::clone(&release),
        Arc::clone(&executions),
        Arc::clone(&cancel_requests),
        Some(Arc::clone(&metrics)),
    );
    let holder_started = started.notified();
    let holder = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            tools
                .execute_with_context(
                    "bounded_sql_query",
                    json!({"sql": "SELECT 1"}),
                    tool_context(CancellationToken::new()),
                )
                .await
        })
    };
    holder_started.await;

    let waiter_token = CancellationToken::new();
    let waiter = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            tools
                .execute_with_context(
                    "bounded_sql_query",
                    json!({"sql": "SELECT 1"}),
                    tool_context(waiter_token),
                )
                .await
        })
    };
    let mut waiter = waiter;
    assert_pending(&mut waiter, "waiter").await;
    let before_release = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &before_release,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "only the holder's wait is recorded while the waiter pends: {before_release:?}"
    );
    assert_eq!(
        samples_in(&before_release, AgentMetricPhase::DatabaseExecute),
        0,
        "no execute settles before any release: {before_release:?}"
    );

    release.notify_waiters();
    wait_for_executions(&executions, 2).await;
    let after_grant = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &after_grant,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        2,
        "the waiter's wait sample appears only after the lease is granted: {after_grant:?}"
    );
    assert_eq!(
        outcome_in(
            &after_grant,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "only the holder's execute has settled: {after_grant:?}"
    );
    assert_pending(&mut waiter, "waiter inside execute").await;

    release.notify_waiters();
    holder
        .await
        .expect("holder task joins")
        .expect("holder query succeeds");
    let waiter_result = waiter
        .await
        .expect("waiter task joins")
        .expect("waiter query succeeds after its lease");
    assert!(waiter_result.is_object(), "waiter received a result");
    let settled = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &settled,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        2,
        "each operation recorded exactly one succeeded wait: {settled:?}"
    );
    assert_eq!(
        outcome_in(
            &settled,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Succeeded
        ),
        2,
        "each operation recorded exactly one succeeded execute: {settled:?}"
    );
    assert_eq!(executions.load(Ordering::SeqCst), 2);
    assert_eq!(cancel_requests.load(Ordering::SeqCst), 0);
}

/// (c) A waiter cancelled while the lease is held records one cancelled
/// DatabaseWait sample and no DatabaseExecute sample at all; the holder's
/// execute still settles once its independent release fires.
#[tokio::test]
async fn cancelled_waiter_records_no_execute_sample() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let executions = Arc::new(AtomicUsize::new(0));
    let cancel_requests = Arc::new(AtomicUsize::new(0));
    let metrics = Arc::new(LocalMetrics::new());
    let tools = lease_tools(
        Arc::clone(&started),
        Arc::clone(&release),
        Arc::clone(&executions),
        Arc::clone(&cancel_requests),
        Some(Arc::clone(&metrics)),
    );
    let holder_started = started.notified();
    let holder = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            tools
                .execute_with_context(
                    "bounded_sql_query",
                    json!({"sql": "SELECT 1"}),
                    tool_context(CancellationToken::new()),
                )
                .await
        })
    };
    holder_started.await;

    let waiter_token = CancellationToken::new();
    let waiter_cancellation = waiter_token.clone();
    let waiter = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            tools
                .execute_with_context(
                    "bounded_sql_query",
                    json!({"sql": "SELECT 1"}),
                    tool_context(waiter_cancellation),
                )
                .await
        })
    };
    let mut waiter = waiter;
    assert_pending(&mut waiter, "waiter").await;
    waiter_token.cancel();
    waiter
        .await
        .expect("waiter task joins")
        .expect_err("cancelled waiter is refused");
    let after_cancel = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &after_cancel,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Cancelled
        ),
        1,
        "the cancelled lease wait is counted once: {after_cancel:?}"
    );
    assert_eq!(
        outcome_in(
            &after_cancel,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the holder's wait is the only succeeded one: {after_cancel:?}"
    );
    assert_eq!(
        samples_in(&after_cancel, AgentMetricPhase::DatabaseExecute),
        0,
        "a cancelled waiter records no execute sample: {after_cancel:?}"
    );
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "the waiter starts no SQL"
    );
    assert_eq!(
        cancel_requests.load(Ordering::SeqCst),
        0,
        "the waiter cannot cancel the holder's source operation"
    );

    release.notify_waiters();
    holder
        .await
        .expect("holder task joins")
        .expect("holder query succeeds");
    let settled = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &settled,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the holder's execute alone settles: {settled:?}"
    );
}

/// (d) A connector whose execute fails records one DatabaseExecute sample
/// with the failed outcome, beside the succeeded lease wait.
#[tokio::test]
async fn failing_connector_execute_records_a_failed_execute_sample() {
    let metrics = Arc::new(LocalMetrics::new());
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(FailingConnector),
            dialect: SqlDialect::Sqlite,
            profile_id: None,
        },
    );
    let tools = DatabaseTools::with_registry(registry, 100, true, None)
        .with_metrics(Some(Arc::clone(&metrics)));
    let result = tools
        .execute_with_context(
            "bounded_sql_query",
            json!({"sql": "SELECT 1"}),
            tool_context(CancellationToken::new()),
        )
        .await;
    assert!(result.is_err(), "the failing connector is reported");
    let snapshot = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the lease was acquired before the execute: {snapshot:?}"
    );
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::DatabaseExecute),
        1,
        "one execute sample: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Failed
        ),
        1,
        "the connector error settles the execute as failed: {snapshot:?}"
    );
}
