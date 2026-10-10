//! Tests for the request-scoped observation collector (slice 3b-2).
//!
//! These drive `DatabaseTools` through the `ToolExecutor` surface with an
//! `ObservationLog` attached, then `drain` the log and assert exactly what an
//! observation may carry — and what it must not.

use super::*;
use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::DatabaseConnector;
use saya_types::{
    ConnectionError, Database, DatabaseProfile, ProfileIdentity, QueryRequest, QueryResult, Schema,
    SchemaTree, SqlDialect, Table,
};
use std::path::Path;
use std::sync::Arc;

use crate::agent::tools::database_tools::{
    DrainedObservations, ObservationLog, ObservationOutcome, ToolObservation,
};
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// A connector whose queries succeed and whose result is fully controllable,
/// so a test can plant a sentinel row value and a known row count.
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
        Err(ConnectionError::schema_failed("nope"))
    }
    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed(
            "syntax error near SENTINELLITERAL",
        ))
    }
}

/// A connector with a non-empty schema, for the schema_discovery test.
struct SchemaConnector {
    table: &'static str,
}

#[async_trait]
impl DatabaseConnector for SchemaConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree {
            databases: vec![Database {
                name: "main".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![Table {
                        name: self.table.into(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                        columns: vec![],
                    }],
                }],
            }],
        })
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

fn identity(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "observations.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/observations-test/connections.toml"),
    )
}

fn entry_with(
    connector: Box<dyn DatabaseConnector>,
    identity: &ProfileIdentity,
) -> ConnectionEntry {
    ConnectionEntry {
        connector,
        dialect: SqlDialect::DuckDb,
        profile_id: Some(identity.as_str().to_string()),
    }
}

fn log_and_tools(
    registry: ConnectionRegistry,
    allow_query_data: bool,
) -> (Arc<ObservationLog>, DatabaseTools) {
    let log = Arc::new(ObservationLog::new());
    let tools = DatabaseTools::with_registry_and_observations(
        registry,
        100,
        allow_query_data,
        None,
        log.clone(),
    );
    (log, tools)
}

fn registry_one(
    connector: Box<dyn DatabaseConnector>,
    identity: &ProfileIdentity,
) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert("primary", entry_with(connector, identity));
    registry
}

// Test 1: a successful bounded_sql_query records the object, column, row count.
// Test 2: a failed query records Failed and no error text.
// Test 3: a denied call records Denied with no objects and no row count.
// Test 4: bounded_sql_query_all across two connections records two observations
// with different profiles.
// Test 5: schema_discovery records an observation with no objects.
// Test 6: a contract tool call records nothing.
// Test 7: the 32 cap holds and drain reports truncation.
// Test 8: no SQL and no result value in a drained observation. The query text
// contains SENTINELLITERAL and the result contains SENTINELROWVALUE; neither
// may appear anywhere in the drained observations' debug output.
// Test 9: with no collector attached, tool behaviour and output are byte-identical
// to today. Attaching a collector cannot change the result a tool returns.
// Test 10: drain empties the log, so a second call returns nothing.

#[path = "observations_tests/collection.rs"]
mod collection;
#[path = "observations_tests/query_outcomes.rs"]
mod query_outcomes;
