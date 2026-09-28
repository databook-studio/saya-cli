//! S11 red tests: `/export --snapshot` writes the captured result with no
//! query at all, `--refresh` (and the legacy form) re-run the last query on
//! its original connection, and the export writer publishes atomically —
//! refusing an existing destination without `--overwrite`, never clobbering
//! a file on failure, and never leaving a temp file behind.

use super::outcome::Dispatch;
use super::query::apply_query_actions;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::export::{
    MAX_EXPORT_BYTES, write_result_overwrite, write_result_with_ceiling,
};
use crate::interactive::tui::sql_task::{Followup, SqlTask, complete};
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use crate::interactive::tui::types::LastQuery;
use crate::interactive::tui::ui_snapshot_tests::unused_runtime;
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
    apply_query_actions(
        SessionAction::Export(request),
        transcript,
        state,
        last_query,
        captured,
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

/// The snapshot writes the captured result itself: no `SqlTask` is
/// dispatched (nothing is re-run) and the file carries the captured
/// columns and rows exactly — not a viewport altered by scroll or
/// `/columns`.
#[test]
fn snapshot_export_never_queries() {
    let dir = temp_dir("snapshot-no-query");
    let path = dir.join("snap.csv");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let captured = captured_of(sample_result());
    let outcome = apply_export_action(
        snapshot_request(&path),
        &mut transcript,
        &mut state,
        &mut None,
        &captured,
    );
    assert!(
        outcome.is_none(),
        "a snapshot export must not dispatch a SqlTask"
    );
    let written = std::fs::read_to_string(&path).expect("snapshot wrote the file");
    assert_eq!(
        written, "id,name\n1,alice\n2,\"bob, jr\"\n",
        "the captured rows are exported exactly: {written}"
    );
    let msg = last_block(&transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Exported 2 row(s) to")
            && msg.contains("from snapshot exec xabc-1")
            && msg.contains("(captured 22:13:20"),
        "the snapshot message names the execution and the capture time: {msg}"
    );
}

/// With nothing captured, the snapshot refuses and says how to recover —
/// it never falls back to re-running anything.
#[test]
fn snapshot_without_capture_says_how_to_recover() {
    let dir = temp_dir("snapshot-no-capture");
    let path = dir.join("snap.csv");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let outcome = apply_export_action(
        snapshot_request(&path),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
    );
    assert!(outcome.is_none(), "no capture, no task");
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert!(
        msg.contains("No captured result to snapshot"),
        "the refusal names the mode: {msg}"
    );
    assert!(
        msg.contains("use /export --refresh"),
        "the refusal names the way out: {msg}"
    );
    assert!(!path.exists(), "nothing was written");
}

/// A refresh re-runs the last query on the connection it originally ran on
/// — `state.profile` must not override the last query's connection.
#[test]
fn refresh_export_preserves_original_connection() {
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", Some("other".to_string()), "model");
    let mut last_query = Some(LastQuery {
        sql: "SELECT id FROM users".to_string(),
        connection: Some("analytics".to_string()),
    });
    // The legacy form (no mode flag) and the explicit --refresh form both
    // dispatch the same re-run.
    for mode in [None, Some(ExportMode::Refresh)] {
        let request = export_request(mode, false, "out.csv");
        let outcome =
            apply_export_action(request, &mut transcript, &mut state, &mut last_query, &None);
        let Some(Dispatch::SqlTask(task)) = outcome else {
            panic!("a refresh dispatches a SqlTask");
        };
        assert_eq!(
            task.profile,
            Some("analytics".to_string()),
            "the original connection is kept"
        );
        assert_eq!(task.sql, "SELECT id FROM users");
        let Followup::Export { request } = &task.followup else {
            panic!("the followup is the export");
        };
        assert_eq!(request.path, "out.csv");
        assert!(!request.overwrite);
        assert_eq!(request.mode, mode);
    }
}

/// The legacy refresh completion, when a capture exists, says the snapshot
/// mode would have exported the result the user already inspected; an
/// explicit `--refresh` never says it (the user chose the re-run).
#[test]
fn legacy_refresh_hint_names_snapshot_only_when_captured() {
    let dir = temp_dir("legacy-hint");
    // A distinct destination per call: without --overwrite the writer
    // refuses an existing file, which is the writer test's business, not
    // this one's.
    let run = |name: &str, mode: Option<ExportMode>, mut captured: Option<CapturedResult>| {
        let task = SqlTask {
            profile: Some("analytics".to_string()),
            sql: "SELECT id, name FROM users".to_string(),
            followup: Followup::Export {
                request: export_request(mode, false, &dir.join(name).to_string_lossy()),
            },
            started_unix_ms: STARTED_UNIX_MS,
        };
        let mut transcript = Transcript::new();
        complete(
            &task,
            TerminalEvent::QueryResult {
                result: sample_result(),
            },
            &mut transcript,
            &mut None,
            &mut captured,
            &unused_runtime(),
        );
        transcript
    };

    // The legacy form with a capture present: the hint names --snapshot.
    let msg = last_block(
        &run("legacy.csv", None, captured_of(sample_result())),
        BlockKind::System,
    )
    .expect("export success is said");
    assert!(
        msg.contains("Exported 2 row(s) to")
            && msg.contains("— refreshed: re-ran the query just now"),
        "the refresh message names the re-run: {msg}"
    );
    assert!(
        msg.contains("(use --snapshot to export the result you already have)"),
        "the legacy form points at --snapshot when a capture exists: {msg}"
    );

    // An explicit --refresh with a capture present: no hint, the user chose.
    let msg = last_block(
        &run(
            "explicit.csv",
            Some(ExportMode::Refresh),
            captured_of(sample_result()),
        ),
        BlockKind::System,
    )
    .expect("export success is said");
    assert!(
        msg.contains("— refreshed: re-ran the query just now") && !msg.contains("--snapshot"),
        "an explicit refresh never suggests --snapshot: {msg}"
    );

    // The legacy form with nothing captured: no pointless hint.
    let msg = last_block(&run("bare.csv", None, None), BlockKind::System)
        .expect("export success is said");
    assert!(
        !msg.contains("--snapshot"),
        "no hint without a capture: {msg}"
    );
}

/// The writer refuses an existing destination unless `--overwrite` is
/// given, and the refusal leaves the existing file byte-for-byte.
#[test]
fn export_refuses_existing_destination_without_overwrite() {
    let dir = temp_dir("refuse-existing");
    let path = dir.join("out.csv");
    std::fs::write(&path, "keep-me").expect("seed the destination");
    let error = write_result_overwrite(&sample_result(), &path, false).unwrap_err();
    assert!(
        error.contains("exists; add --overwrite"),
        "the refusal names the flag: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "keep-me",
        "the existing file is untouched"
    );
    assert_no_temp_files(&dir);
}

/// With `--overwrite` the export replaces the existing file in one rename:
/// the content is fully the new export, never a mix.
#[test]
fn export_overwrite_replaces_atomically() {
    let dir = temp_dir("overwrite");
    let path = dir.join("out.csv");
    std::fs::write(&path, "old").expect("seed the destination");
    let written = write_result_overwrite(&sample_result(), &path, true).unwrap();
    assert_eq!(written, 2);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "id,name\n1,alice\n2,\"bob, jr\"\n",
        "the export fully replaced the old content"
    );
    assert_no_temp_files(&dir);
}

/// A symlink destination is refused even with `--overwrite` — the export
/// never writes through a link to somewhere else.
#[cfg(unix)]
#[test]
fn export_refuses_symlink_destination() {
    let dir = temp_dir("symlink");
    let real = dir.join("real.csv");
    std::fs::write(&real, "real").expect("seed the link target");
    let link = dir.join("link.csv");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    let error = write_result_overwrite(&sample_result(), &link, false).unwrap_err();
    assert!(
        error.contains("symlink"),
        "the refusal names the link: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "real",
        "the link target is untouched"
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link still there")
            .file_type()
            .is_symlink(),
        "the link itself is untouched"
    );
}

/// An encoding failure (here: the oversize ceiling, injected through the
/// test-sized helper) must leave an existing destination byte-for-byte and
/// create nothing.
#[test]
fn failed_export_preserves_destination() {
    let dir = temp_dir("failed-oversize");
    let path = dir.join("out.csv");
    std::fs::write(&path, "keep-me").expect("seed the destination");
    let big = QueryResult {
        columns: vec!["t".to_string()],
        rows: vec![serde_json::json!(["x".repeat(64)])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT t".to_string(),
    };
    // Over the (tiny, injected) ceiling even with --overwrite: the failure
    // is the ceiling, and the destination survives it.
    let error = write_result_with_ceiling(&big, &path, true, 16).unwrap_err();
    assert_eq!(error, "export larger than 32 MiB");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "keep-me",
        "the existing file is untouched on failure"
    );
    // A fresh destination over the ceiling creates nothing — for CSV and,
    // through the serialiser's own writer, for JSON alike.
    let fresh = dir.join("fresh.csv");
    write_result_with_ceiling(&big, &fresh, false, 16).unwrap_err();
    assert!(!fresh.exists(), "no partial file on failure");
    let fresh_json = dir.join("fresh.json");
    write_result_with_ceiling(&big, &fresh_json, false, 16).unwrap_err();
    assert!(!fresh_json.exists(), "no partial JSON file on failure");
    assert_no_temp_files(&dir);
}

/// A publish failure after the guard (here: an unwritable directory, so
/// the private temp file cannot be created) leaves the existing
/// destination byte-for-byte and no temp file behind.
#[cfg(unix)]
#[test]
fn failed_publish_in_unwritable_dir_leaves_no_temp() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("failed-publish");
    let path = dir.join("out.csv");
    std::fs::write(&path, "keep-me").expect("seed the destination");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
        .expect("chmod the dir read-only");
    let error = write_result_overwrite(&sample_result(), &path, true).unwrap_err();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore the dir for cleanup");
    assert!(
        error.contains("temporary"),
        "the failure names the staging step: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "keep-me",
        "the existing file is untouched when staging fails"
    );
    assert_no_temp_files(&dir);
}

/// The snapshot honours a `.json` destination: the captured rows exported
/// as the same pretty JSON objects the re-run path writes.
#[test]
fn snapshot_export_writes_json_exactly() {
    let dir = temp_dir("snapshot-json");
    let path = dir.join("snap.json");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let captured = captured_of(sample_result());
    apply_export_action(
        snapshot_request(&path),
        &mut transcript,
        &mut state,
        &mut None,
        &captured,
    );
    let written = std::fs::read_to_string(&path).expect("snapshot wrote the file");
    assert_eq!(
        written,
        "[\n  {\n    \"id\": 1,\n    \"name\": \"alice\"\n  },\n  {\n    \"id\": 2,\n    \"name\": \"bob, jr\"\n  }\n]",
        "the JSON export is the captured rows, pretty-printed: {written}"
    );
}

/// The production ceiling is the 32 MiB the failure message names.
#[test]
fn max_export_ceiling_is_32_mib() {
    assert_eq!(MAX_EXPORT_BYTES, 32 * 1024 * 1024);
}
