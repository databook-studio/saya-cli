use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::params::BoundParam;

/// A bounded query passed to a database connector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryRequest {
    pub sql: String,
    pub max_rows: usize,
    /// Named parameter bindings for `:name` placeholders in the SQL. Empty
    /// — and omitted from serialization — for parameter-free queries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<BoundParam>,
}

impl QueryRequest {
    pub fn new(sql: impl Into<String>, max_rows: usize) -> Self {
        Self {
            sql: sql.into(),
            max_rows,
            params: Vec::new(),
        }
    }

    /// `new` plus named parameter bindings. The safety layer matches these
    /// names against the SQL's placeholders exactly, in both directions.
    pub fn with_params(sql: impl Into<String>, max_rows: usize, params: Vec<BoundParam>) -> Self {
        Self {
            sql: sql.into(),
            max_rows,
            params,
        }
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

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
