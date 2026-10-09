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

#[path = "result_shape_tests/advertisement.rs"]
mod advertisement;
#[path = "result_shape_tests/safety.rs"]
mod safety;
#[path = "result_shape_tests/shape.rs"]
mod shape;
