use super::super::run_panel::{RunPanel, RunStep};
use super::super::run_worker::RunWorker;
use super::super::stream_events::apply_event;
use super::super::transcript::BlockKind;
use super::super::types::App;
use super::super::ui_snapshot_tests::empty_app;
use saya_agent::AgentEvent;

/// An app with a small session conversation and the panel in the given
/// state. The conversation above and the panel below are what the snapshot
/// shows, so the separation is in the same frame.
pub(crate) fn app_with_panel(panel: RunPanel) -> App {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    app.transcript.push(
        BlockKind::Assistant,
        "The orders table holds 128 rows in catalog.public.orders.",
    );
    app.run_panel = Some(panel);
    app
}

/// A panel with the given steps and lifecycle line — the fields the four
/// screens differ on. The worker handle is real while the run is in flight
/// (a channel nobody sends to), so the panel reads as active.
pub(crate) fn panel(
    run_id: &str,
    goal: &str,
    active: bool,
    status: &str,
    status_is_error: bool,
    steps: Vec<RunStep>,
) -> RunPanel {
    let (tx, rx) = super::super::run_panel::test_channels();
    let mut p = RunPanel::new(
        RunWorker {
            rx,
            cancel: saya_agent::CancellationToken::new(),
        },
        run_id.into(),
        goal.into(),
    );
    if !active {
        // The drive ended: the worker is released, the record stays.
        p.worker = None;
        let _ = tx;
    }
    p.steps = steps;
    p.status = status.into();
    p.status_is_error = status_is_error;
    p.terminated = !active;
    p
}

/// Seeds the episode's own transcript through the seam the events arrive on.
pub(crate) fn episode(panel: &mut RunPanel, events: Vec<AgentEvent>) {
    for event in events {
        apply_event(&mut panel.episode, event, false);
    }
}
