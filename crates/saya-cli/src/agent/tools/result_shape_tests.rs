//! Tests for the `result_shape` tool. The tool runs a bounded read-only query
//! through the same safety-layer path as `bounded_sql_query` and returns only
//! the shape — row count, whether the row cap was hit, and the column names
//! with an inferred type label. The defining invariant: no cell value ever
//! appears in the serialized return value.

use super::*;
use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::{DatabaseConnector, prepare_duckdb_sql};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

/// A connector that returns a single, fixed, mixed-type row so the shape's
/// column names and inferred type labels can be asserted exactly.
struct TypedConnector;

#[async_trait]
impl DatabaseConnector for TypedConnector {
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
            columns: vec![
                "customer".into(),
                "age".into(),
                "balance".into(),
                "active".into(),
            ],
            rows: vec![serde_json::json!(["Alice", 30, 99.5, true])],
            row_count: 1,
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
            rows: vec![serde_json::json!(["SHAPE_LEAK_SENTINEL_42"])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A connector reporting an empty result that still carries column names, so the
/// empty-result test can assert the columns remain present with no row to infer
/// a type from.
struct EmptyColumnsConnector;

#[async_trait]
impl DatabaseConnector for EmptyColumnsConnector {
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
            columns: vec!["customer".into(), "age".into()],
            rows: Vec::new(),
            row_count: 0,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A connector that simulates a real backend's row cap: it returns at most
/// `max_rows` rows and sets `truncated` when the underlying total exceeds it.
struct CappingConnector {
    total: usize,
}

#[async_trait]
impl DatabaseConnector for CappingConnector {
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
        let cap = req.max_rows;
        let returned = self.total.min(cap);
        let truncated = self.total > cap;
        Ok(QueryResult {
            columns: vec!["id".into()],
            rows: (0..returned)
                .map(|i| serde_json::json!([i as i64]))
                .collect(),
            row_count: returned,
            truncated,
            executed_sql: req.sql,
        })
    }
}

/// A connector that routes through the read-only safety layer (as a real
/// connector does), so a write statement is rejected by the policy itself
/// rather than by a fake that merely returns empty.
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
async fn result_shape_reports_the_connector_row_count_and_column_types() {
    let tools = DatabaseTools::new(Some(Box::new(TypedConnector)), 100, true);
    let shape = tools
        .execute(
            "result_shape",
            serde_json::json!({"sql": "SELECT customer, age, balance, active FROM customers"}),
        )
        .await
        .expect("result_shape should succeed");

    assert_eq!(shape["row_count"], 1);
    assert_eq!(shape["truncated"], false);
    let columns = shape["columns"]
        .as_array()
        .expect("columns is an array of name/type objects");
    assert_eq!(columns.len(), 4);
    assert_eq!(columns[0]["name"], "customer");
    assert_eq!(columns[0]["type"], "TEXT");
    assert_eq!(columns[1]["name"], "age");
    assert_eq!(columns[1]["type"], "INTEGER");
    assert_eq!(columns[2]["name"], "balance");
    assert_eq!(columns[2]["type"], "REAL");
    assert_eq!(columns[3]["name"], "active");
    assert_eq!(columns[3]["type"], "BOOLEAN");
}

#[tokio::test]
async fn result_shape_never_leaks_a_planted_cell_value() {
    let tools = DatabaseTools::new(Some(Box::new(SentinelConnector)), 100, true);
    let shape = tools
        .execute(
            "result_shape",
            serde_json::json!({"sql": "SELECT data FROM secrets"}),
        )
        .await
        .expect("result_shape should succeed");

    let serialized = serde_json::to_string(&shape).expect("shape serializes");
    assert!(
        !serialized.contains("SHAPE_LEAK_SENTINEL_42"),
        "a cell value escaped into the shape: {serialized}"
    );
    // The column name and an inferred type label are still present — the shape
    // is useful, only the values are suppressed.
    assert!(serialized.contains("data"));
    assert!(serialized.contains("TEXT"));
}

#[tokio::test]
async fn result_shape_reports_zero_rows_with_columns_still_present() {
    let tools = DatabaseTools::new(Some(Box::new(EmptyColumnsConnector)), 100, true);
    let shape = tools
        .execute(
            "result_shape",
            serde_json::json!({"sql": "SELECT * FROM empty"}),
        )
        .await
        .expect("result_shape should succeed on an empty result");

    assert_eq!(shape["row_count"], 0);
    assert_eq!(shape["truncated"], false);
    let columns = shape["columns"]
        .as_array()
        .expect("columns still present for an empty result");
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0]["name"], "customer");
    assert_eq!(columns[1]["name"], "age");
    // No row to infer a type from: the type label is null, never a value.
    assert_eq!(columns[0]["type"], serde_json::Value::Null);
    assert_eq!(columns[1]["type"], serde_json::Value::Null);
}

#[tokio::test]
async fn result_shape_sets_truncated_when_the_row_cap_is_hit() {
    let tools = DatabaseTools::new(Some(Box::new(CappingConnector { total: 5 })), 3, true);
    let shape = tools
        .execute(
            "result_shape",
            serde_json::json!({"sql": "SELECT id FROM big"}),
        )
        .await
        .expect("result_shape should succeed");

    assert_eq!(shape["row_count"], 3, "row_count is a floor at the cap");
    assert_eq!(
        shape["truncated"], true,
        "hitting the row cap sets truncated"
    );
}

#[tokio::test]
async fn result_shape_refuses_a_write_exactly_as_bounded_sql_query_does() {
    let tools = DatabaseTools::new(Some(Box::new(SafetyConnector)), 100, true);
    let write_sql = serde_json::json!({"sql": "DROP TABLE customers"});

    let shape_err = tools
        .execute("result_shape", write_sql.clone())
        .await
        .expect_err("result_shape must refuse a write statement");
    let query_err = tools
        .execute("bounded_sql_query", write_sql)
        .await
        .expect_err("bounded_sql_query refuses the same write statement");

    assert_eq!(
        shape_err, query_err,
        "result_shape must refuse a write identically to bounded_sql_query"
    );
    assert!(
        shape_err.to_string().contains("read-only safety policy"),
        "the refusal must come from the read-only safety layer: {shape_err}"
    );
}

#[tokio::test]
async fn result_shape_is_refused_when_data_sharing_is_off() {
    let tools = DatabaseTools::new(None, 10, false);
    let error = tools
        .execute("result_shape", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect_err("result_shape must be refused when data sharing is off");
    assert!(
        error.to_string().contains("data sharing is disabled"),
        "got: {error}"
    );
}

#[test]
fn result_shape_is_advertised_with_its_arguments_when_sharing_is_on() {
    let tools = DatabaseTools::definitions(true, false, false);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "result_shape")
        .expect("result_shape is advertised when data sharing is on");
    assert!(tool.read_only);
    assert!(tool.effect.requires_approval);
    assert_eq!(tool.parameters["required"], serde_json::json!(["sql"]));
    assert!(tool.parameters["properties"]["connection"].is_object());
    assert!(tool.parameters["properties"]["sql"].is_object());

    let hidden = DatabaseTools::definitions(false, false, false);
    assert!(
        !hidden.iter().any(|tool| tool.name == "result_shape"),
        "result_shape is not advertised when data sharing is off"
    );
}
