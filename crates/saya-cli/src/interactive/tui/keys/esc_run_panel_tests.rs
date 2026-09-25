use super::*;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::run_panel::{RunPanel, test_channels};
use crate::interactive::tui::run_worker::RunWorker;
use saya_agent::CancellationToken;

/// A panel wired the way the real worker hands one back.
fn app_with_panel(active: bool) -> (App, CancellationToken) {
    let (_tx, rx) = test_channels();
    let cancel = CancellationToken::new();
    let mut app = idle_app();
    let mut panel = RunPanel::new(
        RunWorker {
            rx,
            cancel: cancel.clone(),
        },
        "r-keys-1".into(),
        "goal".into(),
    );
    if !active {
        panel.worker = None;
    }
    app.run_panel = Some(panel);
    (app, cancel)
}

/// Esc cancels an in-flight run: the token fires, the panel stays (it is
/// cancelling), and nothing claims the conversation.
#[test]
fn esc_cancels_an_in_flight_run() {
    let (mut app, cancel) = app_with_panel(true);
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(cancel.is_cancelled(), "the stop cancels the token");
    assert!(app.run_panel.is_some(), "the panel stays while cancelling");
}

/// Esc closes a finished panel; the conversation returns and the durable
/// record stays in /runs.
#[test]
fn esc_closes_a_finished_panel() {
    let (mut app, _cancel) = app_with_panel(false);
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.run_panel.is_none(), "the finished panel closes");
}

/// Phase 5 packet 1: Esc while an agent stream runs asks for a stop — the
/// transcript says "Stop requested" and must not claim the worker already
/// confirmed ("Stopped.").
#[test]
fn esc_says_stop_requested_not_stopped() {
    let (mut app, _cancel) = app_with_panel(false);
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    app.request.stream = Some(crate::interactive::tui::agent::Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "a prompt".into(),
    });
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let texts: Vec<&str> = app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert!(
        texts.iter().any(|text| text.contains("Stop requested")),
        "Esc must say the stop was requested: {texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text.contains("Stopped.")),
        "requesting a stop must not claim the worker confirmed: {texts:?}"
    );
}

/// An agent stream in the conversation cancels first: with both a stream
/// and a finished panel on screen, Esc stops the stream and the panel
/// stays. A run and an agent stream are separate workers, and Esc never
/// closes the panel out from under an active stream.
#[test]
fn esc_cancels_the_agent_stream_before_touching_the_panel() {
    let (mut app, _cancel) = app_with_panel(false);
    let stream =
        crate::interactive::tui::agent::start(crate::interactive::tui::agent::StreamRequest {
            runtime: app.runtime.clone(),
            prompt: "a prompt".into(),
            approval: saya_agent::ApprovalPolicy::ReadOnly,
            policy: saya_agent::SessionPolicy::new(saya_agent::ApprovalPolicy::ReadOnly),
            overrides: crate::agent::runtime::PromptOverrides::default(),
            history: Vec::new(),
            state_db: app.state_db.clone(),
            last_sql: None,
            session: std::sync::Arc::clone(&app.session),
            journal: None,
            agent_mode: saya_agent::AgentMode::Build,
        });
    app.request.stream = Some(stream);
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        app.run_panel.is_some(),
        "the panel is not closed while the stream cancels"
    );
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("the stream cancel posts");
    assert_eq!(
        last.text,
        "Stop requested — waiting for the worker to confirm."
    );
}
