//! Tests for the agent query capture hook (C1): one successful
//! single-connection `bounded_sql_query` hands the TUI the typed result the
//! model saw; fan-out and the probe tools never capture; a failure never
//! captures; an over-budget result refuses whole — never a partial.

use super::*;
use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::{ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use std::sync::{Arc, Mutex};

use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// The events one hook observed, shared with the test through a mutex.
type Observed = Arc<Mutex<Vec<CaptureEvent>>>;

fn observing_hook() -> (CaptureHook, Observed) {
    let observed: Observed = Arc::new(Mutex::new(Vec::new()));
    let sink = observed.clone();
    let hook: CaptureHook = Arc::new(move |event| sink.lock().unwrap().push(event));
    (hook, observed)
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

/// One successful `bounded_sql_query`: the hook receives the typed result the
/// model saw, under the model row cap — the same rows, byte-for-byte.
#[tokio::test]
async fn agent_query_hook_receives_the_rows_the_model_saw() {
    let (connector, dir) = sqlite_fixture().await;
    let (hook, observed) = observing_hook();
    let tools = DatabaseTools::new(Some(connector), 200, true).with_capture(Some(hook));
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id, name FROM users ORDER BY id"}),
        )
        .await
        .expect("query succeeds");

    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one capture event: {events:?}");
    let CaptureEvent::Captured(capture) = &events[0] else {
        panic!("expected a capture, got {:?}", events[0]);
    };
    // The hook's typed result is exactly what the model received.
    let model_result: QueryResult = serde_json::from_value(value).unwrap();
    assert_eq!(capture.result, model_result);
    // The applied cap is the model cap, not the configured 200 — and the
    // 60-row table is capped to it.
    assert_eq!(
        capture.row_cap,
        crate::agent::state_tools::model_row_cap(200)
    );
    assert_eq!(capture.result.rows.len(), 50);
    assert!(capture.result.truncated);
    assert!(capture.started_unix_ms <= capture.finished_unix_ms);
    let _ = std::fs::remove_dir_all(dir);
}

/// When the model omits `connection` (or passes an empty one) the registry
/// resolves the primary: the capture records that resolved name, plus the
/// entry's dialect and profile identity — never the omission.
#[tokio::test]
async fn omitted_connection_records_the_resolved_connection_name() {
    let (hook, observed) = observing_hook();
    let mut registry = ConnectionRegistry::new("warehouse");
    registry.insert("warehouse", scripted_entry());
    let tools = DatabaseTools::with_registry(registry, 100, true, None).with_capture(Some(hook));
    for connection in [None, Some(""), Some("warehouse")] {
        let mut arguments = serde_json::json!({"sql": "SELECT 1"});
        if let Some(connection) = connection {
            arguments["connection"] = serde_json::json!(connection);
        }
        tools
            .execute("bounded_sql_query", arguments)
            .await
            .expect("query succeeds");
    }
    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 3);
    for event in events.iter() {
        let CaptureEvent::Captured(capture) = event else {
            panic!("expected a capture, got {event:?}");
        };
        assert_eq!(capture.connection, "warehouse");
        assert_eq!(
            capture.profile_identity.as_deref(),
            Some("p-capture-identity-sentinel")
        );
        assert_eq!(capture.dialect, SqlDialect::Sqlite);
    }
}

/// Fan-out and the probe tools run SQL through the same state-tools path but
/// must never capture: only single-connection `bounded_sql_query` does.
#[tokio::test]
async fn fanout_and_probe_tools_never_capture() {
    let (hook, observed) = observing_hook();
    let tools = DatabaseTools::new(
        Some(Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"]), serde_json::json!(["y"])],
            row_count: 2,
            truncated: false,
        })),
        100,
        true,
    )
    .with_capture(Some(hook));
    tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect("fan-out succeeds");
    tools
        .execute("schema_discovery", serde_json::json!({}))
        .await
        .expect("schema discovery succeeds");
    tools
        .execute("result_shape", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect("result shape succeeds");
    tools
        .execute("column_health", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect("column health succeeds");
    // A probeable statement, so join_check's two COUNT queries really run.
    tools
        .execute(
            "join_check",
            serde_json::json!({"sql": "SELECT SUM(o.amount) FROM orders o \
                 JOIN items i ON o.id = i.order_id"}),
        )
        .await
        .expect("join check succeeds");
    assert!(
        observed.lock().unwrap().is_empty(),
        "only bounded_sql_query captures"
    );
}

/// A failed query captures nothing: there is no result the model saw.
#[tokio::test]
async fn failed_query_never_captures() {
    let (hook, observed) = observing_hook();
    let tools =
        DatabaseTools::new(Some(Box::new(FailingConnector)), 100, true).with_capture(Some(hook));
    let error = tools
        .execute("bounded_sql_query", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect_err("a connector failure surfaces as a tool Err");
    let _ = error;
    assert!(
        observed.lock().unwrap().is_empty(),
        "a failure never captures"
    );
}

/// Without a hook the executor behaves exactly as before: the same query
/// returns the same value with and without one attached.
#[tokio::test]
async fn no_hook_no_behaviour_change() {
    let query = serde_json::json!({"sql": "SELECT id FROM orders"});
    let script = ScriptedConnector {
        rows: vec![serde_json::json!(["x"])],
        row_count: 1,
        truncated: false,
    };
    let plain = DatabaseTools::new(Some(Box::new(script)), 100, true);
    let plain_value = plain
        .execute("bounded_sql_query", query.clone())
        .await
        .expect("plain path succeeds");
    let (hook, observed) = observing_hook();
    let hooked = DatabaseTools::new(
        Some(Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"])],
            row_count: 1,
            truncated: false,
        })),
        100,
        true,
    )
    .with_capture(Some(hook));
    let hooked_value = hooked
        .execute("bounded_sql_query", query)
        .await
        .expect("hooked path succeeds");
    assert_eq!(plain_value, hooked_value, "the model's value is identical");
    assert_eq!(observed.lock().unwrap().len(), 1);
}

/// Over the accounted budget the hook sends one whole refusal — sql and
/// connection only — and the model still receives its full result; the seam
/// (`capture_event` at a test-sized budget) shows the same decision whole.
#[tokio::test]
async fn over_budget_sends_refusal_not_partial() {
    let (hook, observed) = observing_hook();
    let tools =
        DatabaseTools::new(Some(Box::new(BigCellConnector)), 100, true).with_capture(Some(hook));
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT big", "connection": "primary"}),
        )
        .await
        .expect("the model still receives its result");
    assert_eq!(value["columns"][0], "big", "the full value is unchanged");

    // The seam: the same decision at a test-sized budget — 5 bytes for the
    // cell plus 1 for the column name.
    let entry = ConnectionEntry {
        connector: Box::new(FailingConnector),
        dialect: SqlDialect::Sqlite,
        profile_id: None,
    };
    let executed = ExecutedQuery {
        value: serde_json::json!({}),
        result: QueryResult {
            columns: vec!["c".into()],
            rows: vec![serde_json::json!(["hello"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT 'hello'".into(),
        },
        row_cap: 50,
        started_unix_ms: 1,
        finished_unix_ms: 2,
    };
    assert!(matches!(
        capture_event("SELECT 'hello'", "primary", &entry, &executed, 6),
        CaptureEvent::Captured(_)
    ));
    assert!(matches!(
        capture_event("SELECT 'hello'", "primary", &entry, &executed, 5),
        CaptureEvent::Refused { sql, connection }
            if sql == "SELECT 'hello'" && connection == "primary"
    ));

    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one event: {events:?}");
    assert!(matches!(
        &events[0],
        CaptureEvent::Refused { sql, connection }
            if sql == "SELECT big" && connection == "primary"
    ));
}
