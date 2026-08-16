//! Tests for the request-scoped observation collector (slice 3b-2).
//!
//! These drive `DatabaseTools` through the `ToolExecutor` surface with an
//! `ObservationLog` attached, then `drain` the log and assert exactly what an
//! observation may carry — and what it must not. See
//! `.claude/specs/spec-3b2-observation-collector.md` §5.

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
#[tokio::test]
async fn successful_query_records_object_column_and_row_count() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![serde_json::json!(["x"]), serde_json::json!(["y"])],
                row_count: 2,
                truncated: false,
            }),
            &identity("primary"),
        ),
        true,
    );
    tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id FROM analytics.public.orders"}),
        )
        .await
        .expect("query succeeds");

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 1);
    let obs = &drained.observations[0];
    assert_eq!(obs.tool, "bounded_sql_query");
    assert_eq!(obs.outcome, ObservationOutcome::Succeeded);
    assert_eq!(obs.objects, vec![vec!["analytics", "public", "orders"]]);
    assert_eq!(obs.columns, vec!["id".to_string()]);
    assert_eq!(obs.row_count, Some(2));
    assert_eq!(obs.truncated, Some(false));
    assert!(!obs.references_partial);
    assert_eq!(obs.profile, Some(identity("primary")));
}

// Test 2: a failed query records Failed and no error text.
#[tokio::test]
async fn failed_query_records_failed_without_error_text() {
    let (log, tools) = log_and_tools(
        registry_one(Box::new(FailingConnector), &identity("primary")),
        true,
    );
    let err = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id FROM orders"}),
        )
        .await
        .expect_err("a connector failure surfaces as a tool Err");
    // The tool error string is NOT recorded — only the outcome enum is.
    let _ = err;

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 1);
    let obs = &drained.observations[0];
    assert_eq!(obs.outcome, ObservationOutcome::Failed);
    // The observation carries no error text anywhere — and the connector error
    // string itself contained a sentinel that must not ride along.
    let blob = format!("{obs:?}");
    assert!(!blob.contains("syntax error"), "error text leaked: {blob}");
    assert_eq!(obs.row_count, None);
    assert_eq!(obs.truncated, None);
    // Failed query still records the objects the SQL named (it parsed).
    assert_eq!(obs.objects, vec![vec!["orders"]]);
}

// Test 3: a denied call records Denied with no objects and no row count.
#[tokio::test]
async fn denied_call_records_denied_with_no_objects_or_rows() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            &identity("primary"),
        ),
        false,
    );
    let err = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id FROM orders"}),
        )
        .await
        .expect_err("data-sharing-disabled refuses the tool as Err");
    // The tool error string is not recorded — only the Denied outcome is.
    let _ = err;

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 1);
    let obs = &drained.observations[0];
    assert_eq!(obs.outcome, ObservationOutcome::Denied);
    assert!(obs.objects.is_empty());
    assert!(obs.columns.is_empty());
    assert_eq!(obs.row_count, None);
    assert_eq!(obs.truncated, None);
    assert_eq!(obs.profile, None);
}

// Test 4: bounded_sql_query_all across two connections records two observations
// with different profiles.
#[tokio::test]
async fn fan_out_records_one_observation_per_connection() {
    let primary = identity("primary");
    let warehouse = identity("warehouse");
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        entry_with(
            Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            &primary,
        ),
    );
    registry.insert(
        "warehouse",
        ConnectionEntry {
            connector: Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            dialect: SqlDialect::Postgres,
            profile_id: Some(warehouse.as_str().to_string()),
        },
    );
    let (log, tools) = log_and_tools(registry, true);
    tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT id FROM orders"}),
        )
        .await
        .expect("fan-out succeeds");

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 2);
    let profiles: Vec<_> = drained.observations.iter().map(|o| &o.profile).collect();
    assert!(profiles.contains(&&Some(primary.clone())));
    assert!(profiles.contains(&&Some(warehouse.clone())));
    assert_ne!(
        profiles[0], profiles[1],
        "each observation has its own profile"
    );
}

// Test 5: schema_discovery records an observation with no objects.
#[tokio::test]
async fn schema_discovery_records_no_objects() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(SchemaConnector { table: "orders" }),
            &identity("primary"),
        ),
        true,
    );
    tools
        .execute("schema_discovery", serde_json::json!({}))
        .await
        .expect("schema discovery succeeds");

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 1);
    let obs = &drained.observations[0];
    assert_eq!(obs.tool, "schema_discovery");
    assert_eq!(obs.outcome, ObservationOutcome::Succeeded);
    assert!(obs.objects.is_empty());
    assert!(obs.columns.is_empty());
    assert_eq!(obs.row_count, None);
}

// Test 6: a contract tool call records nothing.
#[tokio::test]
async fn contract_tool_call_records_nothing() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(SchemaConnector { table: "orders" }),
            &identity("primary"),
        ),
        true,
    );
    // contract_search has no store to read, so it returns an empty result — but
    // the point is that it records nothing either way.
    let _ = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .expect("contract tool degrades to empty, never Err");
    let drained = log.drain();
    assert!(
        drained.observations.is_empty(),
        "a contract tool call must not record: {:?}",
        drained.observations
    );
}

// Test 7: the 32 cap holds and drain reports truncation.
#[tokio::test]
async fn cap_holds_and_drain_reports_truncation() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            &identity("primary"),
        ),
        true,
    );
    for _ in 0..40 {
        tools
            .execute(
                "bounded_sql_query",
                serde_json::json!({"sql": "SELECT 1 FROM t"}),
            )
            .await
            .expect("query succeeds");
    }
    let drained = log.drain();
    assert_eq!(drained.observations.len(), 32);
    assert!(drained.truncated);
}

// Test 8: no SQL and no result value in a drained observation. The query text
// contains SENTINELLITERAL and the result contains SENTINELROWVALUE; neither
// may appear anywhere in the drained observations' debug output.
#[tokio::test]
async fn drained_observations_contain_no_sql_or_result_values() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![serde_json::json!(["SENTINELROWVALUE"])],
                row_count: 1,
                truncated: false,
            }),
            &identity("primary"),
        ),
        true,
    );
    tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT 'SENTINELLITERAL' AS label, name FROM orders WHERE token = 'SENTINELLITERAL'"}),
        )
        .await
        .expect("query succeeds");

    let drained = log.drain();
    let blob = format!("{drained:?}");
    assert!(
        !blob.contains("SENTINELLITERAL"),
        "SQL literal leaked into observation: {blob}"
    );
    assert!(
        !blob.contains("SENTINELROWVALUE"),
        "result value leaked into observation: {blob}"
    );
}

// Test 9: with no collector attached, tool behaviour and output are byte-identical
// to today. Attaching a collector cannot change the result a tool returns.
#[tokio::test]
async fn no_collector_means_byte_identical_output() {
    // `ConnectionRegistry` is not `Clone`, so build two identical registries.
    let registry_without = registry_one(
        Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"])],
            row_count: 1,
            truncated: false,
        }),
        &identity("primary"),
    );
    let registry_with = registry_one(
        Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"])],
            row_count: 1,
            truncated: false,
        }),
        &identity("primary"),
    );

    let without = DatabaseTools::with_registry(registry_without, 100, true, None);
    let with_log = Arc::new(ObservationLog::new());
    let with = DatabaseTools::with_registry_and_observations(
        registry_with,
        100,
        true,
        None,
        with_log.clone(),
    );

    let args = serde_json::json!({"sql": "SELECT id FROM orders"});
    let out_without = without
        .execute("bounded_sql_query", args.clone())
        .await
        .expect("without collector");
    let out_with = with
        .execute("bounded_sql_query", args)
        .await
        .expect("with collector");
    assert_eq!(out_without, out_with, "collector must not change output");
    // The collector still recorded — it just did not touch the output.
    assert_eq!(with_log.drain().observations.len(), 1);
}

// Test 10: drain empties the log, so a second call returns nothing.
#[tokio::test]
async fn drain_empties_so_a_turn_cannot_double_count() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            &identity("primary"),
        ),
        true,
    );
    tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT 1 FROM t"}),
        )
        .await
        .unwrap();
    let first = log.drain();
    assert_eq!(first.observations.len(), 1);
    let second = log.drain();
    assert!(second.observations.is_empty(), "a turn cannot double-count");
}

/// A focused unit check on the collector's own drain/truncation contract, kept
/// here so the bound is asserted at the integration surface too.
#[test]
fn drain_report_type_carries_truncation_flag() {
    let drained = DrainedObservations {
        observations: Vec::<ToolObservation>::new(),
        truncated: true,
    };
    assert!(drained.truncated);
    assert!(drained.observations.is_empty());
}
