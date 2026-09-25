use super::super::run_panel::{RunPanel, test_channels};
use super::super::run_worker::{RunMsg, RunWorker};
use super::super::types::App;
use crate::interactive::tui::application::tests_support::idle_app;
use saya_agent::{AgentEvent, ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;
use saya_types::{PauseReason, RunEvent};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A panel wired to a hand-built worker — the exact shape the real spawn
/// hands back, so the drain under test is the drain the loop calls.
pub(crate) fn panel_app(
    run_id: &str,
    goal: &str,
) -> (App, std::sync::mpsc::Sender<RunMsg>, CancellationToken) {
    let (tx, rx) = test_channels();
    let cancel = CancellationToken::new();
    let mut app = idle_app();
    app.run_panel = Some(RunPanel::new(
        RunWorker {
            rx,
            cancel: cancel.clone(),
        },
        run_id.into(),
        goal.into(),
    ));
    (app, tx, cancel)
}

/// Panel separation and pause: the episode transcript stays in its own panel,
/// cancel stops the run and refuses a pending plan, a paused run names its reason.
/// Moved byte-identical from the hub; no snapshots involved.
/// ever reaches the session's conversation, and vice versa.
#[test]
fn an_episodes_transcript_stays_in_its_own_panel() {
    let (mut app, tx, _cancel) = panel_app("r-panel-sep", "explain the schema");
    tx.send(RunMsg::Episode(AgentEvent::tool_requested(
        "bounded_sql_query",
        serde_json::json!({"sql": "SELECT 1", "connection": "analytics"}),
        Some(saya_agent::ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: true,
            local_state: saya_agent::LocalStateEffect::None,
        }),
    )))
    .unwrap();
    tx.send(RunMsg::Episode(AgentEvent::assistant_text(
        "The orders table tracks billing.",
    )))
    .unwrap();
    app.poll_run_panel(false);
    let panel = app.run_panel.as_ref().unwrap();
    assert!(
        !panel.episode.blocks().is_empty(),
        "the episode's events landed in the panel's transcript"
    );
    assert!(
        app.transcript.blocks().is_empty(),
        "the session's conversation never receives the episode"
    );
}

/// Cancel from the panel: the token cancels, the panel says so, and a
/// pending plan approval is refused — a run the user is stopping is never
/// approved mid-stop.
#[test]
fn cancelling_the_panel_stops_the_run_and_refuses_a_pending_plan() {
    let (mut app, tx, cancel) = panel_app("r-panel-cancel", "survey the data");
    let (respond, answer) = tokio::sync::oneshot::channel();
    tx.send(RunMsg::PlanApproval {
        view_text: "run approval — goal: survey the data".into(),
        steps: vec!["ask the data".into()],
        respond,
    })
    .unwrap();
    app.poll_run_panel(false);
    assert!(!cancel.is_cancelled(), "nothing asked to stop yet");

    app.cancel_run_panel();
    assert!(cancel.is_cancelled(), "the panel's stop cancels the token");
    assert!(
        app.run_panel.as_ref().unwrap().cancelling,
        "the panel says it is cancelling"
    );
    assert!(
        !answer.blocking_recv().unwrap(),
        "a pending plan approval is refused by the stop, never approved"
    );
    // Esc closes a finished panel; the durable record stays in /runs.
    app.close_run_panel();
    assert!(app.run_panel.is_none());
}

/// A usage refusal before anything exists — no scopes stated — reaches the
/// panel through the real worker with the run surface's own words, exit 2.
#[test]
fn a_run_without_scopes_refuses_into_the_panel() {
    let runtime = Arc::new(crate::interactive::tui::application::tests_support::unused_runtime());
    let run_id = crate::commands::new_run_id();
    let worker = super::super::run_worker::spawn(super::super::run_worker::RunJob {
        runtime,
        state_db: SqliteStateStore::new(PathBuf::new()),
        format: crate::RenderFormat::Text,
        approval: ApprovalPolicy::ReadOnly,
        profile: None,
        request: crate::commands::RunRequest {
            run_id: run_id.clone(),
            goal: Some("survey the data".into()),
            allow: Vec::new(),
            budget: Vec::new(),
        },
    });
    let mut app = idle_app();
    app.run_panel = Some(RunPanel::new(
        worker,
        run_id.as_str().to_string(),
        "survey the data".into(),
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.run_panel.as_ref().unwrap().is_active() {
        app.poll_run_panel(false);
        assert!(Instant::now() < deadline, "the refusal never landed");
        std::thread::sleep(Duration::from_millis(10));
    }
    let panel = app.run_panel.as_ref().unwrap();
    assert!(panel.status_is_error, "the refusal reads as an error");
    assert!(
        panel.status.contains("--allow"),
        "the run surface's refusal reached the panel: {:?}",
        panel.status
    );
    // Nothing was claimed: no run directory was created for a refused run.
    assert!(
        !panel.terminated,
        "a refusal is not a lifecycle event — nothing ran"
    );
}

/// A paused run names its reason: the panel's lifecycle line carries the
/// shared shaper's wording (`run paused · …`), the same line the wire and
/// `saya run log` render.
#[test]
fn a_paused_run_names_its_reason_in_the_panel() {
    let (mut app, tx, _cancel) = panel_app("r-panel-pause", "budgeted survey");
    tx.send(RunMsg::Event(RunEvent::Paused {
        reason: PauseReason::BudgetExhausted,
    }))
    .unwrap();
    app.poll_run_panel(false);
    let panel = app.run_panel.as_ref().unwrap();
    assert!(panel.terminated, "a pause is a terminal-for-now state");
    assert_eq!(panel.status, "run paused · a declared budget tripped");
}
