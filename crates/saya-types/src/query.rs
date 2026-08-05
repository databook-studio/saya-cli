use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A bounded query passed to a database connector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryRequest {
    pub sql: String,
    pub max_rows: usize,
}

impl QueryRequest {
    pub fn new(sql: impl Into<String>, max_rows: usize) -> Self {
        Self {
            sql: sql.into(),
            max_rows,
        }
    }
}

/// Connector-neutral tabular result. Values are JSON-compatible for CLI output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
    pub row_count: usize,
    pub truncated: bool,
    pub executed_sql: String,
}

impl QueryResult {
    pub fn empty(sql: impl Into<String>) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            truncated: false,
            executed_sql: sql.into(),
        }
    }
}
