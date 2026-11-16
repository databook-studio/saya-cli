use crate::consensus::Candidate;
use saya_types::QueryResult;
use serde_json::Value;

/// Build a `QueryResult` from column names and rows (each row a JSON array).
pub(super) fn result(columns: &[&str], rows: Vec<Value>) -> QueryResult {
    let row_count = rows.len();
    QueryResult {
        columns: columns.iter().map(|s| s.to_string()).collect(),
        rows,
        row_count,
        truncated: false,
        executed_sql: String::new(),
    }
}

pub(super) fn candidate(sql: &str, result: Option<QueryResult>) -> Candidate {
    Candidate {
        sql: sql.to_string(),
        result,
    }
}
