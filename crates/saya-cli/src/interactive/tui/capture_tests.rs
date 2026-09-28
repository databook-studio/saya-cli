//! S10 red tests: after a successful direct `/sql`, the typed result and its
//! `ExecutionEvidence` are captured on `App` — in memory only, never
//! serialized — and the table block names the execution in one final line
//! under its scope line. Failures, `/chart`, `/explain`, and `/export` never
//! touch the capture; an over-budget result refuses visibly.

use super::capture::{self, CapturedResult};
use super::sql_task::{Followup, SqlTask, complete};
use super::transcript::{BlockKind, Transcript};
use super::ui_snapshot_tests::{empty_app, unused_runtime};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_state::SessionState;
use crate::render::TerminalEvent;
use saya_types::{DatabaseProfile, EvidenceSource, ResultScope, SqlDialect};

const STARTED_UNIX_MS: i64 = 1_700_000_000_000;
const SENTINEL: &str = "capture-sentinel-cell-7f3a9c";

/// The snapshot test runtime, plus a resolvable `analytics` profile (SQLite,
/// so `dialect()` needs no connection — matching how `exec::run_sql` resolves
/// the profile before any query).
fn runtime() -> RuntimeConfig {
    let mut runtime = std::sync::Arc::unwrap_or_clone(unused_runtime());
    runtime.connections.profiles.insert(
        "analytics".to_string(),
        DatabaseProfile::Sqlite {
            path: String::new(),
            read_only: true,
        },
    );
    runtime
}

fn result(rows: usize, truncated: bool) -> saya_types::QueryResult {
    saya_types::QueryResult {
        columns: vec!["id".to_string()],
        rows: (0..rows).map(|i| serde_json::json!([i])).collect(),
        row_count: rows,
        truncated,
        executed_sql: "SELECT id FROM t".to_string(),
    }
}

fn sql_task() -> SqlTask {
    SqlTask {
        profile: Some("analytics".to_string()),
        sql: "SELECT id FROM t".to_string(),
        followup: Followup::Sql {
            connection: Some("analytics".to_string()),
        },
        started_unix_ms: STARTED_UNIX_MS,
    }
}

/// Drives `complete` with a `QueryResult` event through the `/sql` follow-up
/// and returns the transcript and the app's capture slot afterwards.
fn complete_sql(result: saya_types::QueryResult) -> (Transcript, Option<CapturedResult>) {
    let mut transcript = Transcript::new();
    let mut captured = None;
    complete(
        &sql_task(),
        TerminalEvent::QueryResult { result },
        &mut transcript,
        &mut None,
        &mut captured,
        &runtime(),
    );
    (transcript, captured)
}

fn seed_capture() -> CapturedResult {
    let r = result(1, false);
    let evidence = capture::direct_sql_evidence(
        Some("analytics"),
        Some("analytics"),
        &r,
        &runtime(),
        STARTED_UNIX_MS,
    )
    .expect("the test runtime resolves the analytics profile");
    CapturedResult {
        result: r,
        evidence,
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

#[test]
fn sql_success_captures_typed_result_and_evidence() {
    let expected = result(3, false);
    let (_, captured) = complete_sql(expected.clone());
    let capture = captured.expect("a successful /sql result is captured");
    assert_eq!(
        capture.result, expected,
        "the capture holds the typed result, rows included"
    );
    let e = &capture.evidence;
    assert_eq!(e.returned_rows, expected.row_count, "evidence counts match");
    assert_eq!(e.max_rows, runtime().resolved.max_rows);
    assert!(!e.truncated);
    assert_eq!(e.connection_label, "analytics");
    assert_eq!(
        e.connection_identity, None,
        "the identity is never recorded"
    );
    assert_eq!(e.dialect, SqlDialect::Sqlite);
    assert_eq!(
        e.started_unix_ms, STARTED_UNIX_MS,
        "dispatch time is the start"
    );
    assert!(e.finished_unix_ms >= STARTED_UNIX_MS);
    assert_eq!(e.source, EvidenceSource::DirectSql);
    assert_eq!(e.scope, ResultScope::Full);
    assert!(!e.execution_id.is_empty());
}

#[test]
fn failed_sql_keeps_previous_capture_untouched() {
    let seed = seed_capture();
    let mut transcript = Transcript::new();
    let mut captured = Some(seed);
    complete(
        &sql_task(),
        TerminalEvent::Error {
            message: "connection refused".into(),
        },
        &mut transcript,
        &mut None,
        &mut captured,
        &runtime(),
    );
    let kept = captured.expect("a failed /sql never clears the previous capture");
    assert_eq!(
        kept.result.row_count, 1,
        "the previous result is still held"
    );
    assert!(
        last_block(&transcript, BlockKind::Error).is_some(),
        "the failure still rendered as an error block"
    );
}

#[test]
fn chart_and_explain_do_not_capture() {
    // /explain renders a plan table; the capture must not move.
    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let explain = SqlTask {
        profile: Some("analytics".to_string()),
        sql: "EXPLAIN SELECT id FROM t".to_string(),
        followup: Followup::Explain,
        started_unix_ms: STARTED_UNIX_MS,
    };
    complete(
        &explain,
        TerminalEvent::QueryResult {
            result: result(2, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
        &runtime(),
    );
    assert_eq!(
        captured
            .as_ref()
            .expect("explain leaves the capture")
            .result
            .row_count,
        1,
        "the previous capture survives /explain"
    );

    // /chart with a chartable result, written to a path that cannot exist so
    // no temp chart is reserved and no browser is opened. Even this
    // successful-render follow-up must leave the capture alone.
    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let chart = SqlTask {
        profile: Some("analytics".to_string()),
        sql: "SELECT id FROM t".to_string(),
        followup: Followup::Chart {
            kind: None,
            path: Some("/nonexistent-saya-test-dir/chart.html".into()),
        },
        started_unix_ms: STARTED_UNIX_MS,
    };
    complete(
        &chart,
        TerminalEvent::QueryResult {
            result: result(2, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
        &runtime(),
    );
    assert_eq!(
        captured
            .as_ref()
            .expect("chart leaves the capture")
            .result
            .row_count,
        1,
        "the previous capture survives /chart"
    );
}

#[test]
fn capture_budget_has_visible_failure() {
    let big = saya_types::QueryResult {
        columns: vec!["text".to_string()],
        rows: vec![serde_json::json!(["x".repeat(200)])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT text FROM t".to_string(),
    };
    // The parameterised walk the production capture uses: a small budget
    // refuses the big result and admits the small one.
    assert!(
        capture::accounted_bytes_within(&big, 64).is_none(),
        "over a small budget the walk refuses"
    );
    assert!(
        capture::accounted_bytes_within(&result(1, false), 64).is_some(),
        "a small result stays within a small budget"
    );

    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let evidence = capture::direct_sql_evidence(
        Some("analytics"),
        Some("analytics"),
        &big,
        &runtime(),
        STARTED_UNIX_MS,
    )
    .expect("resolvable");
    let accounted = capture::accounted_bytes_within(&big, 64);
    capture::capture_within(&mut captured, big, evidence, accounted, &mut transcript);
    assert!(
        captured.is_none(),
        "over budget clears the previous capture"
    );
    let note = last_block(&transcript, BlockKind::System).expect("the refusal is visible");
    assert!(note.contains("Result not captured"), "{note}");
    assert!(
        note.contains("/export --refresh will re-run it"),
        "the refusal names the way out: {note}"
    );
}

#[test]
fn accounting_stops_at_the_budget_boundary() {
    // One 60-char cell over a 100-byte budget fits (60 + column "text" = 64);
    // a second identical row passes it — the walk refuses there, not after
    // measuring everything.
    let row = serde_json::json!(["x".repeat(60)]);
    let one = saya_types::QueryResult {
        columns: vec!["text".to_string()],
        rows: vec![row.clone()],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT text FROM t".to_string(),
    };
    assert_eq!(capture::accounted_bytes_within(&one, 100), Some(64));
    let two = saya_types::QueryResult {
        rows: vec![row.clone(), row],
        row_count: 2,
        ..one.clone()
    };
    assert_eq!(capture::accounted_bytes_within(&two, 100), None);
    // Arrays and objects are walked: a nested 200-char string refuses a
    // 64-byte budget.
    let nested = saya_types::QueryResult {
        columns: vec!["j".to_string()],
        rows: vec![serde_json::json!({"k": ["x".repeat(200)]})],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT j FROM t".to_string(),
    };
    assert_eq!(capture::accounted_bytes_within(&nested, 64), None);
    assert!(
        capture::accounted_bytes(&one).is_some(),
        "the production budget admits a small result"
    );
}

#[test]
fn session_roundtrip_excludes_result_capture() {
    let mut app = empty_app();
    let captured_result = saya_types::QueryResult {
        columns: vec!["value".to_string()],
        rows: vec![serde_json::json!([SENTINEL])],
        row_count: 1,
        truncated: false,
        executed_sql: format!("SELECT '{SENTINEL}'"),
    };
    app.captured = Some(CapturedResult {
        evidence: capture::direct_sql_evidence(
            Some("analytics"),
            Some("analytics"),
            &captured_result,
            &runtime(),
            STARTED_UNIX_MS,
        )
        .expect("resolvable"),
        result: captured_result,
    });
    // The payload the TUI writes is `SessionState::redacted()` (what
    // `queue_session_save` hands the store); serialize exactly that.
    let state = SessionState::new("sess-1", Some("analytics".to_string()), "qwen");
    let payload = state.redacted();
    let json = serde_json::to_string(&payload).expect("the session payload serializes");
    assert!(
        !json.contains(SENTINEL),
        "the session save must not carry captured rows or SQL:\n{json}"
    );
}

#[test]
fn evidence_line_follows_the_scope_line() {
    let (transcript, _) = complete_sql(result(3, false));
    let block = last_block(&transcript, BlockKind::Table).expect("the result renders as a table");
    let last = block.lines().last().expect("the block has lines");
    assert!(
        last.starts_with("direct sql: analytics · 3 rows"),
        "the evidence line is the block's final line: {last:?}"
    );
    assert!(
        block.contains("from analytics · SELECT id FROM t"),
        "the scope line is still present above it:\n{block}"
    );
    assert!(
        !last.contains("SELECT"),
        "the evidence line carries no SQL text: {last:?}"
    );
}

#[test]
fn truncated_result_evidence_says_truncated() {
    // The test runtime caps at 100 rows (unused_runtime's max_rows).
    let (transcript, captured) = complete_sql(result(100, true));
    let cap = captured.expect("a truncated success is still captured");
    assert!(cap.evidence.truncated);
    let last = last_block(&transcript, BlockKind::Table)
        .expect("table")
        .lines()
        .last()
        .expect("lines")
        .to_string();
    assert!(last.contains("100 rows (truncated at 100)"), "{last}");
}

#[test]
fn dialect_unresolvable_does_not_capture_or_say_anything() {
    // The plain snapshot runtime resolves no profiles: with an unresolvable
    // profile there is no evidence, no capture, and nothing extra said — a
    // dialect is never guessed.
    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let mut task = sql_task();
    task.profile = Some("not-a-profile".to_string());
    complete(
        &task,
        TerminalEvent::QueryResult {
            result: result(3, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
        &unused_runtime(),
    );
    let kept = captured.expect("an unresolvable dialect leaves the previous capture");
    assert_eq!(kept.result.row_count, 1);
    let table = last_block(&transcript, BlockKind::Table).expect("the result still renders");
    assert!(
        !table.contains("direct sql"),
        "nothing extra is said: {table:?}"
    );
    assert!(
        last_block(&transcript, BlockKind::System).is_none(),
        "no system note either"
    );
}

#[test]
fn connection_label_falls_back_to_the_task_profile() {
    // No dispatch-time connection: the label falls back to the profile the
    // query ran on.
    let mut transcript = Transcript::new();
    let mut captured = None;
    let task = SqlTask {
        profile: Some("analytics".to_string()),
        sql: "SELECT id FROM t".to_string(),
        followup: Followup::Sql { connection: None },
        started_unix_ms: STARTED_UNIX_MS,
    };
    complete(
        &task,
        TerminalEvent::QueryResult {
            result: result(2, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
        &runtime(),
    );
    let cap = captured.expect("a successful /sql without a captured connection is still captured");
    assert_eq!(cap.evidence.connection_label, "analytics");
}
