//! Tests for the `column_health` tool. The tool runs a bounded read-only query
//! through the same safety-layer path as `bounded_sql_query` and returns only
//! per-column health statistics — nulls, null percentage, distinct count, and
//! numeric zeros. The defining invariant: no cell value ever appears in the
//! serialized return value.

use super::*;
use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::{DatabaseConnector, prepare_duckdb_sql};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

/// A connector returning rows with known nulls, zeros, and distinct values so
/// the health statistics can be asserted exactly.
struct HealthConnector;

#[async_trait]
impl DatabaseConnector for HealthConnector {
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
            columns: vec!["day".into(), "amount".into(), "flag".into()],
            rows: vec![
                serde_json::json!([null, 0, "a"]),
                serde_json::json!([null, 5, "a"]),
                serde_json::json!([null, 0, "b"]),
            ],
            row_count: 3,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A connector that plants a distinctive sentinel value in a cell. The privacy
/// test asserts the sentinel never appears in the tool's serialized output.
struct SentinelConnector;

#[async_trait]
impl DatabaseConnector for SentinelConnector {
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
            columns: vec!["data".into()],
            rows: vec![serde_json::json!(["HEALTH_LEAK_SENTINEL_99"])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A connector that routes through the read-only safety layer (as a real
/// connector does), so a write statement is rejected by the policy itself.
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

#[tokio::test]
async fn column_health_reports_nulls_distinct_and_zeros_on_a_known_result() {
    let tools = DatabaseTools::new(Some(Box::new(HealthConnector)), 100, true);
    let health = tools
        .execute(
            "column_health",
            serde_json::json!({"sql": "SELECT day, amount, flag FROM t"}),
        )
        .await
        .expect("column_health should succeed");

    assert_eq!(health["row_count"], 3);
    assert_eq!(health["truncated"], false);
    let columns = health["columns"]
        .as_array()
        .expect("columns is an array of stat objects");
    assert_eq!(columns.len(), 3);

    assert_eq!(columns[0]["name"], "day");
    assert_eq!(columns[0]["nulls"], 3);
    assert_eq!(columns[0]["null_pct"], 100.0);
    assert_eq!(columns[0]["distinct"], 1);
    assert_eq!(columns[0]["zeros"], 0);

    assert_eq!(columns[1]["name"], "amount");
    assert_eq!(columns[1]["nulls"], 0);
    assert_eq!(columns[1]["null_pct"], 0.0);
    assert_eq!(columns[1]["distinct"], 2);
    assert_eq!(columns[1]["zeros"], 2);

    assert_eq!(columns[2]["name"], "flag");
    assert_eq!(columns[2]["nulls"], 0);
    assert_eq!(columns[2]["null_pct"], 0.0);
    assert_eq!(columns[2]["distinct"], 2);
    assert_eq!(columns[2]["zeros"], 0);
}

#[tokio::test]
async fn column_health_never_leaks_a_planted_cell_value() {
    let tools = DatabaseTools::new(Some(Box::new(SentinelConnector)), 100, true);
    let health = tools
        .execute(
            "column_health",
            serde_json::json!({"sql": "SELECT data FROM secrets"}),
        )
        .await
        .expect("column_health should succeed");

    let serialized = serde_json::to_string(&health).expect("health serializes");
    assert!(
        !serialized.contains("HEALTH_LEAK_SENTINEL_99"),
        "a cell value escaped into the health report: {serialized}"
    );
    assert!(
        serialized.contains("data"),
        "the column name should still be present"
    );
    assert!(
        serialized.contains("\"distinct\":1"),
        "the distinct count should still be present"
    );
}

#[tokio::test]
async fn column_health_is_refused_when_data_sharing_is_off() {
    let tools = DatabaseTools::new(None, 10, false);
    let error = tools
        .execute("column_health", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect_err("column_health must be refused when data sharing is off");
    assert!(
        error.to_string().contains("data sharing is disabled"),
        "got: {error}"
    );
}

#[tokio::test]
async fn column_health_refuses_a_write_exactly_as_bounded_sql_query_does() {
    let tools = DatabaseTools::new(Some(Box::new(SafetyConnector)), 100, true);
    let write_sql = serde_json::json!({"sql": "DROP TABLE customers"});

    let health_err = tools
        .execute("column_health", write_sql.clone())
        .await
        .expect_err("column_health must refuse a write statement");
    let query_err = tools
        .execute("bounded_sql_query", write_sql)
        .await
        .expect_err("bounded_sql_query refuses the same write statement");

    assert_eq!(
        health_err, query_err,
        "column_health must refuse a write identically to bounded_sql_query"
    );
    assert!(
        health_err.to_string().contains("read-only safety policy"),
        "the refusal must come from the read-only safety layer: {health_err}"
    );
}

#[test]
fn column_health_is_advertised_with_its_arguments_when_sharing_is_on() {
    let tools = DatabaseTools::definitions(true, false, false);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "column_health")
        .expect("column_health is advertised when data sharing is on");
    assert!(tool.read_only);
    assert!(tool.effect.requires_approval);
    assert_eq!(tool.parameters["required"], serde_json::json!(["sql"]));
    assert!(tool.parameters["properties"]["connection"].is_object());
    assert!(tool.parameters["properties"]["sql"].is_object());

    let hidden = DatabaseTools::definitions(false, false, false);
    assert!(
        !hidden.iter().any(|tool| tool.name == "column_health"),
        "column_health is not advertised when data sharing is off"
    );
}
