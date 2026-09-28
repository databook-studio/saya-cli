//! C2/D12: the snapshot and report labels for an agent capture — the
//! model-limited wording on success, and the refusal that names why the
//! latest promoted agent query's rows are not held (never the /sql-only
//! wording when the gap says an agent query was promoted).

use super::apply_query_actions;
use crate::agent::tools::AgentCapture;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::capture::{CapturedResult, agent_evidence};
use crate::interactive::tui::capture_agent::CaptureGap;
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use crate::slash::{ExportMode, ExportRequest, ReportRequest};
use saya_types::{QueryResult, SqlDialect};

const EXEC_SQL: &str = "SELECT id FROM users";

/// A captured agent result with its evidence, as the pairing leaves the slot.
fn agent_captured(rows: usize, row_cap: usize) -> Option<CapturedResult> {
    let capture = AgentCapture {
        sql: EXEC_SQL.to_string(),
        connection: "analytics".to_string(),
        profile_identity: Some("prof-identity-1".to_string()),
        dialect: SqlDialect::Sqlite,
        result: QueryResult {
            columns: vec!["id".to_string()],
            rows: (0..rows).map(|i| serde_json::json!([i])).collect(),
            row_count: rows,
            truncated: false,
            executed_sql: EXEC_SQL.to_string(),
        },
        row_cap,
        started_unix_ms: 1_700_000_000_000,
        finished_unix_ms: 1_700_000_000_005,
    };
    let evidence = agent_evidence(&capture);
    Some(CapturedResult {
        result: capture.result,
        evidence,
    })
}

fn snapshot_request(path: &std::path::Path) -> ExportRequest {
    ExportRequest {
        mode: Some(ExportMode::Snapshot),
        overwrite: false,
        path: path.to_string_lossy().into_owned(),
    }
}

fn report_request(path: &std::path::Path) -> ReportRequest {
    ReportRequest {
        path: path.to_string_lossy().into_owned(),
        rows: Some(2),
        overwrite: false,
    }
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
        "saya-query-capture-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("query capture test dir");
    dir
}

/// The snapshot of an agent capture says what it is: the rows the agent saw,
/// model-limited — never plain "Exported N row(s)".
#[test]
fn snapshot_of_agent_capture_is_labelled_model_limited() {
    let dir = temp_dir("agent-snapshot");
    let path = dir.join("snap.csv");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let outcome = apply_query_actions(
        SessionAction::Export(snapshot_request(&path)),
        &mut transcript,
        &mut state,
        &mut None,
        &agent_captured(2, 100),
        None,
    );
    assert!(outcome.is_none(), "a snapshot never dispatches a query");
    let msg = last_block(&transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Exported 2 row(s) the agent saw to"),
        "the message says whose rows they are: {msg}"
    );
    assert!(msg.contains("from snapshot exec"), "{msg}");
    assert!(
        msg.contains(
            "(model-limited: the full result may be larger — use --refresh for a full read)"
        ),
        "the message states the scope and the way to a full read: {msg}"
    );
}

/// With nothing captured right after a promoted agent query, the refusal
/// names the gap — the budget when the capture was refused, nothing when no
/// capture arrived — and never the /sql-only wording. With no gap at all the
/// established wording stands.
#[test]
fn snapshot_after_uncaptured_agent_query_names_the_gap() {
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let request = SessionAction::Export(snapshot_request(std::path::Path::new(
        "/nonexistent-saya-c2/never.csv",
    )));
    apply_query_actions(
        request.clone(),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
        Some(CaptureGap::OverBudget),
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert_eq!(
        msg,
        "The latest query's rows were not captured (larger than 32 MiB). \
         Use /export --refresh to re-run it.",
        "the refusal names the budget: {msg}"
    );
    apply_query_actions(
        request.clone(),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
        Some(CaptureGap::Missing),
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert_eq!(
        msg, "The latest query's rows were not captured. Use /export --refresh to re-run it.",
        "no reason is invented for a missing capture: {msg}"
    );
    apply_query_actions(request, &mut transcript, &mut state, &mut None, &None, None);
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert!(
        msg.contains("No captured result to snapshot"),
        "with no gap the established wording stands: {msg}"
    );
}

/// The report of an agent capture names the scope twice: in the file's
/// provenance and in the success message.
#[test]
fn report_of_agent_capture_names_scope() {
    let dir = temp_dir("agent-report");
    let path = dir.join("report.md");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    apply_query_actions(
        SessionAction::Report(report_request(&path)),
        &mut transcript,
        &mut state,
        &mut None,
        &agent_captured(2, 100),
        None,
    );
    let written = std::fs::read_to_string(&path).expect("the report was written");
    assert!(
        written.contains("Scope: model-limited (first 100 rows)"),
        "the provenance names the scope: {written}"
    );
    let msg = last_block(&transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("model-limited: the agent saw only the first 100 rows"),
        "the success message states the scope: {msg}"
    );
}
