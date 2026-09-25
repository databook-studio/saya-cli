//! Exemplar snapshot test.
//!
//! `render_event` turns a `TerminalEvent` into terminal / JSON output. The exact
//! shape of that output is tedious to assert by hand and easy to regress, so we pin
//! it with `insta`. Review every snapshot change like code: `cargo insta review`.

use saya_cli::{RenderFormat, TerminalEvent, render_event, render_run_event};
use saya_types::{PauseReason, QueryResult, RunEvent, RunFailureCode};
use serde_json::json;

fn sample_query_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".into(), "name".into()],
        rows: vec![json!([1, "alpha"]), json!([2, "beta"])],
        row_count: 2,
        truncated: true,
        executed_sql: "SELECT id, name FROM t LIMIT 2".into(),
    }
}

#[test]
fn text_render_of_query_result_is_stable() {
    let event = TerminalEvent::QueryResult {
        result: sample_query_result(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn json_render_of_query_result_is_stable() {
    let event = TerminalEvent::QueryResult {
        result: sample_query_result(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Json).stdout);
}

// --- RunEvent NDJSON snapshots: the lifecycle lines the run wire streams. ---
//
// One `RunEvent` per line, tagged `"type"` — the same bytes the journal
// writes (`crate::render_run::journal_line`). The snapshots pin the wire's
// exact shape so a renamed field is a reviewable diff, not a silent break of
// every consumer.

#[test]
fn run_event_ndjson_started_is_stable() {
    insta::assert_snapshot!(render_run_event(&RunEvent::RunStarted, RenderFormat::Ndjson).stdout);
}

#[test]
fn run_event_ndjson_step_started_is_stable() {
    let event = RunEvent::StepStarted { step: 1 };
    insta::assert_snapshot!(render_run_event(&event, RenderFormat::Ndjson).stdout);
}

#[test]
fn run_event_ndjson_step_completed_is_stable() {
    let event = RunEvent::StepCompleted { step: 1 };
    insta::assert_snapshot!(render_run_event(&event, RenderFormat::Ndjson).stdout);
}

#[test]
fn run_event_ndjson_paused_is_stable() {
    let event = RunEvent::Paused {
        reason: PauseReason::BudgetExhausted,
    };
    insta::assert_snapshot!(render_run_event(&event, RenderFormat::Ndjson).stdout);
}

#[test]
fn run_event_ndjson_completed_is_stable() {
    insta::assert_snapshot!(render_run_event(&RunEvent::Completed, RenderFormat::Ndjson).stdout);
}

#[test]
fn run_event_ndjson_failed_is_stable() {
    let event = RunEvent::Failed {
        code: RunFailureCode::SafetyQuery,
    };
    insta::assert_snapshot!(render_run_event(&event, RenderFormat::Ndjson).stdout);
}

#[test]
fn run_event_ndjson_cancelled_is_stable() {
    insta::assert_snapshot!(render_run_event(&RunEvent::Cancelled, RenderFormat::Ndjson).stdout);
}

#[test]
fn run_event_ndjson_usage_is_stable() {
    let event = RunEvent::Usage {
        endpoint: "primary".into(),
        tokens: Some(120),
        turns: Some(3),
        tool_calls: None,
        cached_input_tokens: Some(90),
        cache_creation_input_tokens: None,
    };
    insta::assert_snapshot!(render_run_event(&event, RenderFormat::Ndjson).stdout);
}

#[test]
fn run_event_ndjson_downloaded_bytes_is_stable() {
    let event = RunEvent::DownloadedBytes { bytes: 97 };
    insta::assert_snapshot!(render_run_event(&event, RenderFormat::Ndjson).stdout);
}
