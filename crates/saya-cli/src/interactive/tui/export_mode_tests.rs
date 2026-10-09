//! S11 red tests: `/export --snapshot` writes the captured result with no
//! query at all, `--refresh` (and the legacy form) re-run the last query on
//! its original connection, and the export writer publishes atomically —
//! refusing an existing destination without `--overwrite`, never clobbering
//! a file on failure, and never leaving a temp file behind.

use super::outcome::Dispatch;
use super::query::apply_query_actions;
use crate::interactive::session::commands::SessionAction;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::application::tests_support::unused_runtime;
use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::export::{
    MAX_EXPORT_BYTES, write_result_overwrite, write_result_with_ceiling,
};
use crate::interactive::tui::sql_task::{Followup, SqlTask, complete};
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use crate::interactive::tui::types::LastQuery;
use crate::render::TerminalEvent;
use crate::slash::{ExportMode, ExportRequest};
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, SqlDialect,
};

const STARTED_UNIX_MS: i64 = 1_700_000_000_000;

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".to_string(), "name".to_string()],
        rows: vec![
            serde_json::json!([1, "alice"]),
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
            finished_unix_ms: STARTED_UNIX_MS + 5,
            source: EvidenceSource::DirectSql,
        },
    )
}

fn captured_of(result: QueryResult) -> Option<CapturedResult> {
    let evidence = evidence(&result);
    Some(CapturedResult { result, evidence })
}

fn export_request(mode: Option<ExportMode>, overwrite: bool, path: &str) -> ExportRequest {
    ExportRequest {
        mode,
        overwrite,
        path: path.to_string(),
    }
}

fn snapshot_request(path: &std::path::Path) -> ExportRequest {
    export_request(Some(ExportMode::Snapshot), false, &path.to_string_lossy())
}

/// Dispatches an `Export` action through the real query-follower arm and
/// returns the outcome.
#[allow(clippy::too_many_arguments)]
fn apply_export_action(
    request: ExportRequest,
    transcript: &mut Transcript,
    state: &mut SessionState,
    last_query: &mut Option<LastQuery>,
    captured: &Option<CapturedResult>,
) -> Option<Dispatch> {
    let rt = unused_runtime();
    apply_query_actions(
        &mut Some(SessionAction::Export(request)),
        transcript,
        state,
        last_query,
        captured,
        None,
        &rt,
    )
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-export-mode-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("export test dir");
    dir
}

fn last_block(transcript: &Transcript, kind: BlockKind) -> Option<String> {
    transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.kind == kind)
        .map(|b| b.text.clone())
}

fn assert_no_temp_files(dir: &std::path::Path) {
    let strays: Vec<_> = std::fs::read_dir(dir)
        .expect("dir readable")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.') || name.ends_with(".tmp"))
        .collect();
    assert!(
        strays.is_empty(),
        "a failed export must leave no temp file, found: {strays:?}"
    );
}
#[path = "export_mode_tests/encoding.rs"]
mod encoding;
#[path = "export_mode_tests/publication.rs"]
mod publication;
#[path = "export_mode_tests/snapshot.rs"]
mod snapshot;
