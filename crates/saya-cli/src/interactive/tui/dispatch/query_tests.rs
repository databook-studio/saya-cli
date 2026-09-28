//! S12 tests for the `/report` dispatch handler: it reads the captured
//! result only — no query, no task — and says how to recover when nothing
//! is captured.

use super::super::outcome::Dispatch;
use super::apply_query_actions;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use crate::interactive::tui::types::LastQuery;
use crate::slash::ReportRequest;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, SqlDialect,
};

const STARTED_UNIX_MS: i64 = 1_700_000_000_000;

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".to_string(), "name".to_string()],
        rows: vec![
            serde_json::json!([1, "sentinel-alice"]),
            serde_json::json!([2, "bob"]),
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

fn captured_of(result: QueryResult) -> Option<CapturedResult> {
    let evidence = evidence(&result);
    Some(CapturedResult { result, evidence })
}

fn report_request(path: &str, rows: Option<usize>, overwrite: bool) -> ReportRequest {
    ReportRequest {
        path: path.to_string(),
        rows,
        overwrite,
    }
}

/// Dispatches a `Report` action through the real query-follower arm and
/// returns the outcome.
#[allow(clippy::too_many_arguments)]
fn apply_report_action(
    request: ReportRequest,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
    captured: &Option<CapturedResult>,
) -> Option<Dispatch> {
    apply_query_actions(
        SessionAction::Report(request),
        transcript,
        state,
        last_query,
        captured,
    )
}

fn last_block(transcript: &Transcript, kind: BlockKind) -> Option<String> {
    transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.kind == kind)
        .map(|b| b.text.clone())
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-report-dispatch-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("report dispatch test dir");
    dir
}

/// `/report` writes from the capture only: no `SqlTask` is dispatched even
/// when a last query exists, and the report carries the captured rows and
/// SQL.
#[test]
fn report_never_queries() {
    let dir = temp_dir("never-queries");
    let path = dir.join("report.md");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let mut last_query = Some(LastQuery {
        sql: "SELECT id FROM users".to_string(),
        connection: Some("analytics".to_string()),
    });
    let captured = captured_of(sample_result());
    let outcome = apply_report_action(
        report_request(&path.to_string_lossy(), Some(2), false),
        &mut transcript,
        &mut state,
        &mut last_query,
        &captured,
    );
    assert!(outcome.is_none(), "/report must not dispatch a SqlTask");
    let written = std::fs::read_to_string(&path).expect("the report was written");
    assert!(
        written.contains("sentinel-alice"),
        "the rows come from the capture: {written}"
    );
    assert!(
        written.contains("SELECT id, name FROM users"),
        "the SQL is the captured one: {written}"
    );
    let sha = captured
        .as_ref()
        .expect("capture")
        .evidence
        .submitted_sql_sha256
        .clone();
    assert!(
        written.contains(&format!("Submitted SQL sha256: {sha}")),
        "the provenance names the captured execution: {written}"
    );
    let msg = last_block(&transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Wrote report to") && msg.contains("(2 rows included)"),
        "the success message counts the rows: {msg}"
    );
}

/// With rows omitted the success message says so.
#[test]
fn report_rows_omitted_message_says_so() {
    let dir = temp_dir("rows-omitted");
    let path = dir.join("report.md");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let captured = captured_of(sample_result());
    apply_report_action(
        report_request(&path.to_string_lossy(), None, false),
        &mut transcript,
        &mut state,
        &mut None,
        &captured,
    );
    let msg = last_block(&transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("(rows omitted)"),
        "the omitted-rows success message is said: {msg}"
    );
}

/// With nothing captured, `/report` refuses and says how to recover — it
/// never falls back to re-running anything.
#[test]
fn report_without_capture_says_how_to_recover() {
    let dir = temp_dir("no-capture");
    let path = dir.join("report.md");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let outcome = apply_report_action(
        report_request(&path.to_string_lossy(), None, false),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
    );
    assert!(outcome.is_none(), "no capture, no task");
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert!(
        msg.contains("No captured result to report"),
        "the refusal names the situation: {msg}"
    );
    assert!(
        msg.contains("Run the query with /sql first"),
        "the refusal names the way in: {msg}"
    );
    assert!(
        msg.contains("captures last only for this session"),
        "the refusal names the lifetime: {msg}"
    );
    assert!(!path.exists(), "nothing was written");
}

/// The dispatch keeps the destination guard: an existing report needs
/// `--overwrite`, and the refusal leaves the file byte-for-byte.
#[test]
fn report_refuses_existing_destination_without_overwrite() {
    let dir = temp_dir("existing");
    let path = dir.join("report.md");
    std::fs::write(&path, "keep-me").expect("seed the destination");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let captured = captured_of(sample_result());
    apply_report_action(
        report_request(&path.to_string_lossy(), Some(2), false),
        &mut transcript,
        &mut state,
        &mut None,
        &captured,
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert!(
        msg.contains("exists; add --overwrite"),
        "the refusal names the flag: {msg}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("destination readable"),
        "keep-me",
        "the existing file is untouched"
    );
}
