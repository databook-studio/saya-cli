//! S12 tests: the Markdown report built from the captured result — exact
//! layout, value neutralisation, bounds, and the atomic writer entry.

use super::super::write_report;
use super::*;
use crate::slash::MAX_REPORT_ROWS;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, SqlDialect,
};

const STARTED_UNIX_MS: i64 = 1_700_000_000_000;
const GENERATED_UNIX_MS: i64 = 1_700_000_005_000;

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".to_string(), "name".to_string()],
        rows: vec![
            serde_json::json!([1, "sentinel-alice"]),
            serde_json::json!([2, "bob, jr"]),
        ],
        row_count: 2,
        truncated: false,
        executed_sql: "SELECT id, name FROM users".to_string(),
    }
}

fn evidence(result: &QueryResult) -> ExecutionEvidence {
    ExecutionEvidence::for_result(
        result,
        ExecutionEvidenceArgs {
            execution_id: "xabc-1".to_string(),
            connection_label: "analytics".to_string(),
            connection_identity: None,
            dialect: SqlDialect::Sqlite,
            max_rows: 100,
            started_unix_ms: STARTED_UNIX_MS,
            finished_unix_ms: STARTED_UNIX_MS + 2_000,
            source: EvidenceSource::DirectSql,
        },
    )
}

fn render(rows: Option<usize>, result: &QueryResult) -> String {
    let evidence = evidence(result);
    render_report(&ReportInput {
        result,
        evidence: &evidence,
        rows,
        generated_unix_ms: GENERATED_UNIX_MS,
    })
    .expect("a small report renders")
}

include!("markdown_tests/rendering.rs");

include!("markdown_tests/writer.rs");

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-report-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("report test dir");
    dir
}
