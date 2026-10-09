//! Tests for the execution-evidence contract (`saya-types::evidence`).

use super::*;
use crate::dialect::SqlDialect;
use crate::query::QueryResult;

const SELECT_ONE_SHA256: &str = "e004ebd5b5532a4b85984a62f8ad48a81aa3460c1ca07701f386135d72cdecf5";

fn simple_result(sql: &str) -> QueryResult {
    QueryResult {
        columns: vec!["?column?".to_owned()],
        rows: vec![serde_json::json!(1)],
        row_count: 12,
        truncated: false,
        executed_sql: sql.to_owned(),
    }
}

fn base_args() -> ExecutionEvidenceArgs {
    ExecutionEvidenceArgs {
        execution_id: "x1a2b3c-4".to_owned(),
        connection_label: "warehouse".to_owned(),
        connection_identity: Some("postgres://ops@db.internal:5432/prod".to_owned()),
        dialect: SqlDialect::Postgres,
        max_rows: 1000,
        started_unix_ms: 1_790_000_000_000,
        finished_unix_ms: 1_790_000_000_900,
        source: EvidenceSource::DirectSql,
    }
}

fn evidence() -> ExecutionEvidence {
    ExecutionEvidence::for_result(&simple_result("SELECT 1"), base_args())
}

#[path = "evidence_tests/construction.rs"]
mod construction;
#[path = "evidence_tests/execution_id.rs"]
mod execution_id;
#[path = "evidence_tests/human_line.rs"]
mod human_line;
#[path = "evidence_tests/params.rs"]
mod params;
#[path = "evidence_tests/serialization.rs"]
mod serialization;
