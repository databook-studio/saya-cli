//! The plugin's query-capture fixtures: a seeded SQLite connection, a connector
//! whose result the capture path must refuse, an observing hook, and the chat
//! plugin built over them.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_connectors::{ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

use super::tests::{chat_session, scratch};
use super::{ChatDatabaseFlags, DatabasePlugin};
use crate::agent::tools::{CaptureEvent, CaptureHook, DatabaseTools};
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// The connection's opaque profile identity, so a capture can be told to have
/// recorded the entry it ran on.
pub(super) const PROFILE_IDENTITY: &str = "p-plugin-capture";

/// The events one hook observed, shared with the test through a mutex.
pub(super) type Observed = Arc<Mutex<Vec<CaptureEvent>>>;

pub(super) fn observing_hook() -> (CaptureHook, Observed) {
    let observed: Observed = Arc::new(Mutex::new(Vec::new()));
    let sink = observed.clone();
    let hook: CaptureHook = Arc::new(move |event| sink.lock().unwrap().push(event));
    (hook, observed)
}

/// A seeded SQLite file, opened read-only: 60 rows through the model's own row
/// cap, so a capture that holds them is visibly the capped model view.
pub(super) async fn sqlite_entry(name: &str) -> ConnectionEntry {
    let path = scratch(name).join("capture.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .expect("the fixture database creates");
    sqlx::query("CREATE TABLE IF NOT EXISTS users (id INTEGER, name TEXT)")
        .execute(&seed)
        .await
        .expect("the fixture table creates");
    let values = (0..60)
        .map(|id| format!("({id}, 'user {id}')"))
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(&format!("INSERT INTO users VALUES {values}"))
        .execute(&seed)
        .await
        .expect("the fixture rows insert");
    seed.close().await;
    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("the fixture database opens");
    ConnectionEntry {
        connector: Box::new(connector),
        dialect: SqlDialect::Sqlite,
        profile_id: Some(PROFILE_IDENTITY.to_owned()),
    }
}

/// A connector whose one cell crosses the loop's result cap, so the model's
/// view of it is truncated and the capture must refuse.
pub(super) struct OversizedCellConnector;

#[async_trait]
impl DatabaseConnector for OversizedCellConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
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
                saya_agent::AgentLimits::default().context_byte_budget + 1
            )])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// The chat surface's plugin over one connection, with the capture hook the
/// front drives (`None` for a caller that drives no preview).
pub(super) fn chat_plugin(entry: ConnectionEntry, capture: Option<CaptureHook>) -> DatabasePlugin {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert("primary", entry);
    DatabasePlugin::for_chat(
        Arc::new(DatabaseTools::with_registry(registry, 200, true, None)),
        ChatDatabaseFlags {
            allow_query_data: true,
            has_state_store: false,
            permit_candidate_writes: false,
        },
        chat_session(),
        capture,
    )
}
