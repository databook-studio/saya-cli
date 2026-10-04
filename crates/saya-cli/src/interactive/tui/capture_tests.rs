//! S10 red tests: after a successful direct `/sql`, the typed result and its
//! `ExecutionEvidence` are captured on `App` — in memory only, never
//! serialized — and the table block names the execution in one final line
//! under its scope line. Failures, `/chart`, `/explain`, and `/export` never
//! touch the capture; an over-budget result refuses visibly.

use super::capture::{self, CapturedResult};
use super::sql_task::{Followup, SqlTask, complete};
use super::transcript::{BlockKind, Transcript};
use super::ui_snapshot_tests::unused_runtime;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::sql_execution::SqlExecution;
use crate::render::TerminalEvent;
use saya_types::{DatabaseProfile, EvidenceSource, ResultScope, SqlDialect};

#[cfg(test)]
#[path = "capture_save_tests.rs"]
mod save_roundtrip;

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
    SqlTask::new(
        Some("analytics".to_string()),
        "SELECT id FROM t".to_string(),
        Followup::Sql {
            connection: Some("analytics".to_string()),
        },
    )
}

/// Plants a synthetic executed envelope on the task so `complete` builds
/// evidence from the EXECUTED source, as the worker would have written it.
fn plant_envelope(task: &SqlTask, result: &saya_types::QueryResult, cap: usize) {
    let binding = SqlExecution::binding_for(
        task.profile.as_deref().unwrap_or("analytics"),
        "test-identity",
        SqlDialect::Sqlite,
        cap,
    );
    task.plant_execution_for_test(result, binding);
}

/// Drives `complete` with a `QueryResult` event through the `/sql` follow-up
/// and returns the transcript and the app's capture slot afterwards.
fn complete_sql(result: saya_types::QueryResult) -> (Transcript, Option<CapturedResult>) {
    let mut transcript = Transcript::new();
    let mut captured = None;
    let task = sql_task();
    plant_envelope(&task, &result, runtime().resolved.max_rows);
    complete(
        &task,
        TerminalEvent::QueryResult { result },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    (transcript, captured)
}

fn seed_capture() -> CapturedResult {
    let r = result(1, false);
    let task = sql_task();
    plant_envelope(&task, &r, runtime().resolved.max_rows);
    let evidence = capture::direct_sql_evidence_from_execution(Some("analytics"), &take(&task));
    CapturedResult {
        result: r,
        evidence,
    }
}

fn take(task: &SqlTask) -> crate::interactive::sql_execution::SqlExecution {
    task.take_execution().expect("planted envelope is taken")
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
        e.connection_identity,
        Some("test-identity".to_string()),
        "the identity is the executed binding's"
    );
    assert_eq!(e.dialect, SqlDialect::Sqlite);
    assert!(
        e.started_unix_ms <= e.finished_unix_ms,
        "the planted envelope timestamps never run backwards"
    );
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
    let explain = SqlTask::new(
        Some("analytics".to_string()),
        "EXPLAIN SELECT id FROM t".to_string(),
        Followup::Explain,
    );
    complete(
        &explain,
        TerminalEvent::QueryResult {
            result: result(2, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
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
    let chart = SqlTask::new(
        Some("analytics".to_string()),
        "SELECT id FROM t".to_string(),
        Followup::Chart {
            kind: None,
            path: Some("/nonexistent-saya-test-dir/chart.html".into()),
        },
    );
    complete(
        &chart,
        TerminalEvent::QueryResult {
            result: result(2, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
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
    let envelope_task = sql_task();
    plant_envelope(&envelope_task, &big, runtime().resolved.max_rows);
    let evidence =
        capture::direct_sql_evidence_from_execution(Some("analytics"), &take(&envelope_task));
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
    // An empty slot — no envelope was written — means no evidence, no
    // capture, and nothing extra said: a dialect is never guessed.
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
    let task = SqlTask::new(
        Some("analytics".to_string()),
        "SELECT id FROM t".to_string(),
        Followup::Sql { connection: None },
    );
    let r = result(2, false);
    plant_envelope(&task, &r, runtime().resolved.max_rows);
    complete(
        &task,
        TerminalEvent::QueryResult { result: r },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    let cap = captured.expect("a successful /sql without a captured connection is still captured");
    assert_eq!(cap.evidence.connection_label, "analytics");
}

#[test]
fn a_successful_completion_without_an_envelope_produces_no_evidence() {
    // An empty slot — the worker never wrote an envelope — renders the
    // table with NO evidence line and captures nothing: `complete` never
    // derives provenance from the live runtime.
    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let task = sql_task();
    let rows = result(3, false);
    complete(
        &task,
        TerminalEvent::QueryResult {
            result: rows.clone(),
        },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    assert!(
        captured.is_some(),
        "the previous capture survives an envelope-less success"
    );
    assert_eq!(
        captured.as_ref().expect("kept").result.row_count,
        1,
        "no new capture replaces it"
    );
    let table = last_block(&transcript, BlockKind::Table).expect("the result still renders");
    assert!(
        !table.contains("direct sql"),
        "no evidence line is synthesized: {table:?}"
    );
    assert!(
        table.contains("from analytics · SELECT id FROM t"),
        "the scope line still renders: {table:?}"
    );
}

#[test]
fn two_tasks_keep_their_own_envelopes() {
    // Two tasks with different bindings, completed in the opposite order:
    // each completion reads its own slot, never the other's.
    let first = SqlTask::new(
        Some("analytics".to_string()),
        "SELECT 1".to_string(),
        Followup::Sql {
            connection: Some("analytics".to_string()),
        },
    );
    let second = SqlTask::new(
        Some("analytics".to_string()),
        "SELECT 2".to_string(),
        Followup::Sql {
            connection: Some("analytics".to_string()),
        },
    );
    let one = result(1, false);
    let two = result(2, false);
    let binding_one = SqlExecution::binding_for("analytics", "identity-one", SqlDialect::Sqlite, 7);
    let binding_two = SqlExecution::binding_for("analytics", "identity-two", SqlDialect::Sqlite, 9);
    first.plant_execution_for_test(&one, binding_one);
    second.plant_execution_for_test(&two, binding_two);

    // Opposite order: second completes first.
    let mut transcript = Transcript::new();
    let mut captured = None;
    complete(
        &second,
        TerminalEvent::QueryResult { result: two },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    let cap = captured.expect("the second task captures");
    assert_eq!(
        cap.evidence.connection_identity,
        Some("identity-two".to_string())
    );
    assert_eq!(cap.evidence.max_rows, 9);

    let mut transcript = Transcript::new();
    let mut captured = None;
    complete(
        &first,
        TerminalEvent::QueryResult { result: one },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    let cap = captured.expect("the first task captures");
    assert_eq!(
        cap.evidence.connection_identity,
        Some("identity-one".to_string())
    );
    assert_eq!(cap.evidence.max_rows, 7);
}
