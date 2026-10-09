//! Tests for the `join_check` tool. The tool builds a fan-out probe for the
//! statement, runs two COUNT(*) statements through the bounded read-only path,
//! and reports whether the join multiplied or dropped rows. The defining
//! invariants: no cell value ever appears in the serialized return value, a
//! fanned-out join is flagged, a clean 1:1 join is not, a row-dropping join is
//! reported, and a statement with no sound probe returns `applicable: false`.

use super::*;
use async_trait::async_trait;
use saya_connectors::{DatabaseConnector, prepare_duckdb_sql};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

#[path = "join_check_tests/advertising.rs"]
mod advertising;
#[path = "join_check_tests/fan_out_counts.rs"]
mod fan_out_counts;
#[path = "join_check_tests/privacy.rs"]
mod privacy;
#[path = "join_check_tests/refusals.rs"]
mod refusals;

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
