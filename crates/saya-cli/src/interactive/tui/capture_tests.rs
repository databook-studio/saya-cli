//! S10 red tests: after a successful direct `/sql`, the typed result and its
//! `ExecutionEvidence` are captured on `App` — in memory only, never
//! serialized — and the table block names the execution in one final line
//! under its scope line. Failures, `/chart`, `/explain`, and `/export` never
//! touch the capture; an over-budget result refuses visibly.

use super::capture as capture_api;
use super::capture::CapturedResult;
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
    let evidence = capture_api::direct_sql_evidence_from_execution(Some("analytics"), &take(&task));
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
mod budget;
mod capture;
mod evidence;
mod non_capture;
