//! The run panel's behaviors, driven through the seams the event loop uses:
//! the worker's channel (`run_worker::spawn`'s shape), `poll_run_panel` —
//! the drain the loop calls every tick — and the panel's cancel. A real
//! worker is spawned once, against a provider that cannot be reached, so the
//! "run is a worker task" claim is tested end to end rather than asserted.

use super::run_panel::{RunPanel, RunStepStatus, test_channels};
use super::run_worker::{RunMsg, RunOutcome, RunWorker};
use super::types::App;
use crate::interactive::tui::application::tests_support::idle_app;
use saya_agent::{AgentEvent, ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;
use saya_types::{PauseReason, RunEvent};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A panel wired to a hand-built worker — the exact shape the real spawn
/// hands back, so the drain under test is the drain the loop calls.
fn panel_app(
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

/// Red test 1: a run driven from the TUI renders its steps, and the step
/// list updates as steps complete — through the same drain the event loop
/// calls, from the approval ask that seeds the plan to the final lifecycle
/// event. The session's conversation must stay empty the whole time.
#[test]
fn a_run_driven_from_the_tui_renders_its_steps_and_updates_as_they_complete() {
    let (mut app, tx, _cancel) = panel_app("r-panel-steps", "survey the data");
    let (respond, answer) = tokio::sync::oneshot::channel();
    // The plan binds: the approval ask carries the step goals the panel seeds.
    tx.send(RunMsg::PlanApproval {
        view_text: "run approval — goal: survey the data".into(),
        steps: vec!["ask the data".into(), "write the note".into()],
        respond,
    })
    .unwrap();
    tx.send(RunMsg::Event(RunEvent::RunStarted)).unwrap();
    app.poll_run_panel(false);
    {
        let panel = app.run_panel.as_ref().unwrap();
        assert_eq!(panel.steps.len(), 2, "the plan's steps seeded the list");
        assert_eq!(panel.steps[0].goal, "ask the data");
        assert_eq!(panel.steps[0].status, RunStepStatus::Pending);
        assert_eq!(panel.status, "run started");
        assert!(
            app.transcript.blocks().is_empty(),
            "the run does not touch the conversation"
        );
    }

    // Step 1 runs and completes; step 2 starts. The panel advances with each
    // drain, exactly as the loop would show it.
    tx.send(RunMsg::Event(RunEvent::StepStarted { step: 0 }))
        .unwrap();
    app.poll_run_panel(false);
    assert_eq!(
        app.run_panel.as_ref().unwrap().steps[0].status,
        RunStepStatus::Running
    );
    assert_eq!(app.run_panel.as_ref().unwrap().status, "step 1 started");

    tx.send(RunMsg::Event(RunEvent::StepCompleted { step: 0 }))
        .unwrap();
    tx.send(RunMsg::Event(RunEvent::StepStarted { step: 1 }))
        .unwrap();
    app.poll_run_panel(false);
    let panel = app.run_panel.as_ref().unwrap();
    assert_eq!(panel.steps[0].status, RunStepStatus::Completed);
    assert!(
        panel.steps[0].elapsed.is_some(),
        "the completed step froze its elapsed"
    );
    assert!(
        panel.steps[0].started.is_none(),
        "no live clock lingers on a done step"
    );
    assert_eq!(panel.steps[1].status, RunStepStatus::Running);
    assert_eq!(panel.status, "step 2 started");
    // The modal the panel is showing is answerable — only an explicit yes
    // approves, and the drive's oneshot carries the decision.
    app.answer_plan_approval(true);
    assert!(
        answer.blocking_recv().unwrap(),
        "the modal's yes reaches the drive's decision"
    );

    // The run completes and the drive ends; the panel keeps the record.
    tx.send(RunMsg::Event(RunEvent::StepCompleted { step: 1 }))
        .unwrap();
    tx.send(RunMsg::Event(RunEvent::Completed)).unwrap();
    tx.send(RunMsg::Done(RunOutcome {
        code: 0,
        message: String::new(),
    }))
    .unwrap();
    app.poll_run_panel(false);
    let panel = app.run_panel.as_ref().unwrap();
    assert!(panel.terminated, "a terminal event marks the run over");
    assert_eq!(panel.status, "run completed");
    assert!(!panel.is_active(), "the worker is released once it ends");
    assert!(
        app.transcript.blocks().is_empty(),
        "still nothing in the conversation"
    );
}

/// Red test 2: the UI stays responsive while a run executes. A worker
/// behaves like the real one — an event, then quiet work, then the end — and
/// every `poll_run_panel` (the loop's per-tick drain) returns immediately
/// while the run is in flight.
#[test]
fn the_event_loop_is_not_blocked_while_a_run_executes() {
    let (mut app, tx, _cancel) = panel_app("r-panel-resp", "long job");
    std::thread::spawn(move || {
        tx.send(RunMsg::Event(RunEvent::StepStarted { step: 0 }))
            .unwrap();
        std::thread::sleep(Duration::from_millis(250));
        tx.send(RunMsg::Done(RunOutcome {
            code: 0,
            message: String::new(),
        }))
        .unwrap();
    });
    // The drain happens while the worker is still running: it must not wait
    // on the worker. The worker may not have sent its first event yet, so
    // poll until it lands — every poll in the loop must be prompt, which is
    // the responsiveness claim itself.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let t0 = Instant::now();
        app.poll_run_panel(false);
        assert!(
            t0.elapsed() < Duration::from_millis(100),
            "the per-tick drain must not block on the worker (took {:?})",
            t0.elapsed()
        );
        if app.run_panel.as_ref().unwrap().status == "step 1 started" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the worker's first event never landed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // The run is still in flight — nothing blocked, nothing finished yet.
    assert!(app.run_panel.as_ref().unwrap().is_active());

    // Give the worker time to finish, then one more prompt drain applies it.
    std::thread::sleep(Duration::from_millis(250));
    let t1 = Instant::now();
    app.poll_run_panel(false);
    assert!(
        t1.elapsed() < Duration::from_millis(100),
        "draining the end must be as prompt as draining progress"
    );
    assert!(
        !app.run_panel.as_ref().unwrap().is_active(),
        "the run's end was applied by a non-blocking poll"
    );
}

/// Red test 5: the episode's transcript is the panel's own — it renders
/// through the same seam the conversation uses, but nothing it receives
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
    let worker = super::run_worker::spawn(super::run_worker::RunJob {
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

/// The real worker, end to end, against a provider that cannot be reached:
/// the run claims its directory, the event loop keeps polling promptly while
/// the worker fails, and the panel ends with the run surface's own words.
#[tokio::test]
async fn a_panel_run_that_cannot_reach_its_provider_fails_into_the_panel() {
    let _env = crate::commands::RUNS_DIR.lock().await;
    let root = std::env::temp_dir().join(format!(
        "saya-run-panel-worker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("runs")).unwrap();
    let previous = std::env::var_os("SAYA_RUNS_DIR");
    // SAFETY: the runs-dir lock above serializes every unit test that points
    // the process-global variable at a private root.
    unsafe { std::env::set_var("SAYA_RUNS_DIR", root.join("runs")) };

    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let mut config = crate::interactive::tui::application::tests_support::unused_runtime();
    // An orchestrator endpoint is resolved but unreachable: the plan proposal
    // fails at the provider layer, which is the failure the panel must show.
    config.resolved.endpoints.insert(
        saya_config::ORCHESTRATOR_ROLE.to_string(),
        saya_config::ResolvedEndpoint {
            name: saya_config::ORCHESTRATOR_ROLE.to_string(),
            provider: saya_config::AiProvider::Ollama,
            model: "test-model".into(),
            base_url: Some("http://127.0.0.1:9/".into()),
            api_key: None,
        },
    );
    let runtime = Arc::new(config);
    let run_id = crate::commands::new_run_id();
    let worker = super::run_worker::spawn(super::run_worker::RunJob {
        runtime,
        state_db: store,
        format: crate::RenderFormat::Text,
        approval: ApprovalPolicy::ReadOnly,
        profile: None,
        request: crate::commands::RunRequest {
            run_id: run_id.clone(),
            goal: Some("survey the data".into()),
            allow: vec!["none".into()],
            budget: Vec::new(),
        },
    });
    let mut app = idle_app();
    app.run_panel = Some(RunPanel::new(
        worker,
        run_id.as_str().to_string(),
        "survey the data".into(),
    ));

    // The event loop's posture, exercised for real: every poll is prompt,
    // however long the worker takes, until the run settles.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let tick = Instant::now();
        app.poll_run_panel(false);
        assert!(
            tick.elapsed() < Duration::from_millis(250),
            "the per-tick poll must never block on the run (took {:?})",
            tick.elapsed()
        );
        if !app.run_panel.as_ref().unwrap().is_active() {
            break;
        }
        assert!(Instant::now() < deadline, "the run never settled");
        std::thread::sleep(Duration::from_millis(50));
    }
    let panel = app.run_panel.as_ref().unwrap();
    assert!(
        panel.status.contains("could not bind a plan"),
        "the run surface's own words reached the panel: {:?}",
        panel.status
    );
    assert!(panel.status_is_error, "a failed run reads as an error");
    // The claim was real: the run's journal holds its first event, under the
    // runs root the panel's worker resolved.
    let journal = saya_harness::journal::Journal::open(root.join("runs").join(run_id.as_str()));
    let events = journal.read().unwrap();
    assert_eq!(events.first(), Some(&RunEvent::RunStarted));

    match previous {
        Some(value) => {
            // SAFETY: same serialized scope as above.
            unsafe { std::env::set_var("SAYA_RUNS_DIR", value) };
        }
        None => {
            // SAFETY: same serialized scope as above.
            unsafe { std::env::remove_var("SAYA_RUNS_DIR") };
        }
    }
    let _ = std::fs::remove_dir_all(root);
}
