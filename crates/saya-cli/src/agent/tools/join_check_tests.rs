//! Tests for the `join_check` tool. The tool builds a fan-out probe for the
//! statement, runs two COUNT(*) statements through the bounded read-only path,
//! and reports whether the join multiplied or dropped rows. The defining
//! invariants: no cell value ever appears in the serialized return value, a
//! fanned-out join is flagged, a clean 1:1 join is not, a row-dropping join is
//! reported, and a statement with no sound probe returns `applicable: false`.

use super::*;
use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::{DatabaseConnector, prepare_duckdb_sql};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

/// A connector that returns configurable counts for the probe's two COUNT(*)
/// statements. It distinguishes the joined statement (contains "JOIN") from the
/// base statement (does not).
struct CountConnector {
    joined: i64,
    base: i64,
}

#[async_trait]
impl DatabaseConnector for CountConnector {
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
        let count = if req.sql.to_ascii_uppercase().contains("JOIN") {
            self.joined
        } else {
            self.base
        };
        Ok(QueryResult {
            columns: vec!["n".into()],
            rows: vec![serde_json::json!([count])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A connector that plants a distinctive sentinel in the count cell, so the
/// privacy test can assert the sentinel never reaches the serialized output.
struct SentinelCountConnector;

#[async_trait]
impl DatabaseConnector for SentinelCountConnector {
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
            columns: vec!["n".into()],
            rows: vec![serde_json::json!(["JOIN_CHECK_SENTINEL_77"])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A connector that routes through the read-only safety layer, so a write
/// statement is rejected by the policy at execution time.
struct SafetyConnector;

#[async_trait]
impl DatabaseConnector for SafetyConnector {
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
        let sql = prepare_duckdb_sql(&req.sql, req.max_rows)?;
        Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            truncated: false,
            executed_sql: sql,
        })
    }
}

const JOIN_SQL: &str = "SELECT SUM(o.amount) FROM orders o \
     JOIN items i ON o.id = i.order_id";

#[tokio::test]
async fn join_check_flags_a_fanned_out_join() {
    let tools = DatabaseTools::new(
        Some(Box::new(CountConnector {
            joined: 100,
            base: 50,
        })),
        100,
        true,
    );
    let result = tools
        .execute("join_check", serde_json::json!({"sql": JOIN_SQL}))
        .await
        .expect("join_check should succeed on a join with an aggregate");

    assert_eq!(result["applicable"], true);
    assert_eq!(result["joined_rows"], 100);
    assert_eq!(result["base_rows"], 50);
    assert_eq!(result["fanned_out"], true);
    assert_eq!(result["dropped_rows"], false);
}

#[tokio::test]
async fn join_check_clears_a_clean_one_to_one_join() {
    let tools = DatabaseTools::new(
        Some(Box::new(CountConnector {
            joined: 50,
            base: 50,
        })),
        100,
        true,
    );
    let result = tools
        .execute("join_check", serde_json::json!({"sql": JOIN_SQL}))
        .await
        .expect("join_check should succeed");

    assert_eq!(result["applicable"], true);
    assert_eq!(result["joined_rows"], 50);
    assert_eq!(result["base_rows"], 50);
    assert_eq!(result["fanned_out"], false);
    assert_eq!(result["dropped_rows"], false);
}

#[tokio::test]
async fn join_check_reports_a_row_dropping_join() {
    let tools = DatabaseTools::new(
        Some(Box::new(CountConnector {
            joined: 30,
            base: 50,
        })),
        100,
        true,
    );
    let result = tools
        .execute("join_check", serde_json::json!({"sql": JOIN_SQL}))
        .await
        .expect("join_check should succeed");

    assert_eq!(result["applicable"], true);
    assert_eq!(result["joined_rows"], 30);
    assert_eq!(result["base_rows"], 50);
    assert_eq!(result["fanned_out"], false);
    assert_eq!(result["dropped_rows"], true);
}

#[tokio::test]
async fn join_check_returns_not_applicable_when_no_sound_probe_can_be_built() {
    let tools = DatabaseTools::new(
        Some(Box::new(CountConnector { joined: 0, base: 0 })),
        100,
        true,
    );
    let result = tools
        .execute(
            "join_check",
            serde_json::json!({"sql": "SELECT SUM(x) FROM single_table"}),
        )
        .await
        .expect("join_check should succeed with applicable: false");

    assert_eq!(result["applicable"], false);
    assert!(
        result["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "a reason must accompany applicable: false"
    );
}

#[tokio::test]
async fn join_check_never_leaks_a_planted_cell_value() {
    let tools = DatabaseTools::new(Some(Box::new(SentinelCountConnector)), 100, true);
    let result = tools
        .execute("join_check", serde_json::json!({"sql": JOIN_SQL}))
        .await
        .expect("join_check should succeed (returning applicable: false)");

    let serialized = serde_json::to_string(&result).expect("result serializes");
    assert!(
        !serialized.contains("JOIN_CHECK_SENTINEL_77"),
        "a cell value escaped into the join_check output: {serialized}"
    );
}

#[tokio::test]
async fn join_check_is_refused_when_data_sharing_is_off() {
    let tools = DatabaseTools::new(None, 10, false);
    let error = tools
        .execute("join_check", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect_err("join_check must be refused when data sharing is off");
    assert!(
        error.to_string().contains("data sharing is disabled"),
        "got: {error}"
    );
}

#[tokio::test]
async fn join_check_refuses_a_write_exactly_as_bounded_sql_query_does() {
    let tools = DatabaseTools::new(Some(Box::new(SafetyConnector)), 100, true);
    let write_sql = serde_json::json!({"sql": "DROP TABLE customers"});

    let join_err = tools
        .execute("join_check", write_sql.clone())
        .await
        .expect_err("join_check must refuse a write statement");
    let query_err = tools
        .execute("bounded_sql_query", write_sql)
        .await
        .expect_err("bounded_sql_query refuses the same write statement");

    assert_eq!(
        join_err, query_err,
        "join_check must refuse a write identically to bounded_sql_query"
    );
    assert!(
        join_err.to_string().contains("read-only safety policy"),
        "the refusal must come from the read-only safety layer: {join_err}"
    );
}

#[test]
fn join_check_is_advertised_with_its_arguments_when_sharing_is_on() {
    let tools = DatabaseTools::definitions(true, false, false);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "join_check")
        .expect("join_check is advertised when data sharing is on");
    assert!(tool.read_only);
    assert!(tool.effect.requires_approval);
    assert_eq!(tool.parameters["required"], serde_json::json!(["sql"]));
    assert!(tool.parameters["properties"]["connection"].is_object());
    assert!(tool.parameters["properties"]["sql"].is_object());

    let hidden = DatabaseTools::definitions(false, false, false);
    assert!(
        !hidden.iter().any(|tool| tool.name == "join_check"),
        "join_check is not advertised when data sharing is off"
    );
}
