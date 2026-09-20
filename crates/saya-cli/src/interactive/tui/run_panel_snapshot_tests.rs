//! Deterministic snapshots of the run panel — the composed screen with the
//! panel docked below the session conversation, at real widths — so the
//! step list's layout, the live status line, and the episode panel's
//! separation are caught in `cargo test` in milliseconds.
//!
//! The pattern is `ui_snapshot_tests.rs`: the `App` is built directly as a
//! struct literal (no store, database, or provider), the episode transcript
//! is driven through `stream_events::apply_event` (the natural seam), and
//! the frame renders through the real `ui::draw` onto a
//! `ratatui::backend::TestBackend`. The buffer view strips colour but keeps
//! trailing whitespace. Elapsed figures are constructed `Duration`s — the
//! boundary events froze them — and the run clock is left unset so no
//! render depends on wall time.
//!
//! Four screens, one per state the panel can be caught in: a running run, a
//! paused run with its reason, a completed run, and a failed run. Each
//! carries a small session turn above the panel, so the separation the
//! milestone promises — the episode's transcript in its own panel, the
//! session's conversation untouched — is visible in the same frame.

use std::time::Duration;

use saya_agent::AgentEvent;

use super::run_panel::{RunStep, RunStepStatus};
use super::ui_snapshot_tests::{fixed_status, render_buffer};

#[cfg(test)]
#[path = "run_panel_snapshot_support.rs"]
mod support;

use support::{app_with_panel, episode, panel};

/// Screen 1 — a running run: step 1 done in 12s, step 2 in flight at 3s,
/// step 3 pending, the lifecycle line on the shared shaper's wording, and
/// the episode's own transcript underneath.
#[test]
fn a_running_run_shows_its_steps_status_and_episode() {
    let mut p = panel(
        "r-snap-1",
        "survey the data quality",
        true,
        "step 2 started",
        false,
        vec![
            RunStep {
                goal: "profile the tables".into(),
                status: RunStepStatus::Completed,
                elapsed: Some(Duration::from_secs(12)),
                started: None,
            },
            RunStep {
                goal: "compare row counts".into(),
                status: RunStepStatus::Running,
                elapsed: Some(Duration::from_secs(3)),
                started: None,
            },
            RunStep {
                goal: "write the survey note".into(),
                status: RunStepStatus::Pending,
                elapsed: None,
                started: None,
            },
        ],
    );
    episode(
        &mut p,
        vec![
            AgentEvent::tool_requested(
                "bounded_sql_query",
                serde_json::json!({"sql": "SELECT status, count(*) FROM orders GROUP BY status",
                               "connection": "analytics"}),
                Some(saya_agent::ToolEffect {
                    database_data: true,
                    external_side_effect: false,
                    requires_approval: true,
                    local_state: saya_agent::LocalStateEffect::None,
                }),
            ),
            AgentEvent::ToolCompleted {
                name: "bounded_sql_query".into(),
                summary: "4 rows".into(),
            },
            AgentEvent::assistant_text("Step one profiled the tables; counts look even."),
        ],
    );
    let buffer = render_buffer(&app_with_panel(p), &fixed_status(), 100, 30);
    insta::assert_snapshot!(buffer);
}

/// Screen 2 — a paused run names its reason: the shared shaper's line is
/// the panel's status, and the failed step shows its frozen elapsed.
#[test]
fn a_paused_run_names_its_reason() {
    let mut p = panel(
        "r-snap-2",
        "audit the ledger",
        false,
        "run paused · a step kept failing past the bounded retries",
        false,
        vec![
            RunStep {
                goal: "pull the ledger".into(),
                status: RunStepStatus::Completed,
                elapsed: Some(Duration::from_secs(8)),
                started: None,
            },
            RunStep {
                goal: "reconcile the totals".into(),
                status: RunStepStatus::Failed,
                elapsed: Some(Duration::from_secs(4)),
                started: None,
            },
        ],
    );
    episode(
        &mut p,
        vec![AgentEvent::assistant_text(
            "The totals did not reconcile; the step kept failing.",
        )],
    );
    let buffer = render_buffer(&app_with_panel(p), &fixed_status(), 100, 30);
    insta::assert_snapshot!(buffer);
}

/// Screen 3 — a completed run: every step done, the shaper's completion
/// line, and the episode tail still readable.
#[test]
fn a_completed_run_shows_every_step_done() {
    let mut p = panel(
        "r-snap-3",
        "survey the data quality",
        false,
        "run completed",
        false,
        vec![
            RunStep {
                goal: "profile the tables".into(),
                status: RunStepStatus::Completed,
                elapsed: Some(Duration::from_secs(12)),
                started: None,
            },
            RunStep {
                goal: "compare row counts".into(),
                status: RunStepStatus::Completed,
                elapsed: Some(Duration::from_secs(9)),
                started: None,
            },
            RunStep {
                goal: "write the survey note".into(),
                status: RunStepStatus::Completed,
                elapsed: Some(Duration::from_secs(3)),
                started: None,
            },
        ],
    );
    episode(
        &mut p,
        vec![AgentEvent::assistant_text(
            "The survey note is in the workspace.",
        )],
    );
    let buffer = render_buffer(&app_with_panel(p), &fixed_status(), 100, 30);
    insta::assert_snapshot!(buffer);
}

/// Screen 4 — a failed run: the shared shaper's failure line, in error
/// styling, with the step that failed last.
#[test]
fn a_failed_run_says_what_failed() {
    let mut p = panel(
        "r-snap-4",
        "audit the ledger",
        false,
        "run failed: the provider or agent layer failed",
        true,
        vec![
            RunStep {
                goal: "pull the ledger".into(),
                status: RunStepStatus::Completed,
                elapsed: Some(Duration::from_secs(6)),
                started: None,
            },
            RunStep {
                goal: "reconcile the totals".into(),
                status: RunStepStatus::Failed,
                elapsed: Some(Duration::from_secs(2)),
                started: None,
            },
        ],
    );
    episode(
        &mut p,
        vec![AgentEvent::assistant_text(
            "The ledger pull stopped mid-way.",
        )],
    );
    let app = app_with_panel(p);
    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    insta::assert_snapshot!(buffer);
}

/// Phase 5 packet 1: a stopped run says "Run stopped." — never the engine's
/// "run cancelled" line — and reads as ended, not failed.
#[test]
fn a_stopped_run_says_run_stopped_not_run_cancelled() {
    let mut p = panel(
        "r-snap-stop",
        "survey the data",
        true,
        "",
        false,
        Vec::new(),
    );
    p.cancelling = true;
    let cancelling = render_buffer(&app_with_panel(p), &fixed_status(), 100, 30);
    assert!(
        cancelling.contains("Stop requested"),
        "the local ack must say Stop requested:\n{cancelling}"
    );
    let mut stopped = panel(
        "r-snap-stop",
        "survey the data",
        false,
        "Run stopped.",
        false,
        Vec::new(),
    );
    stopped.cancelling = false;
    let buffer = render_buffer(&app_with_panel(stopped), &fixed_status(), 100, 30);
    assert!(
        buffer.contains("Run stopped."),
        "the confirmation must say Run stopped.:\n{buffer}"
    );
    assert!(
        !buffer.contains("run cancelled"),
        "the shared shaper's line must not reach the screen:\n{buffer}"
    );
}
