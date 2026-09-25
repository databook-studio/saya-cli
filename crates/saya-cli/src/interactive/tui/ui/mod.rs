//! Rendering for the TUI: transcript, status bar, input box, and overlays.
//! Kept separate from the event loop so styling can evolve on its own.

pub(crate) mod chrome;
pub(super) mod input;
pub(super) mod overlays;
pub(crate) mod surface;
pub(crate) mod theme;

use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;
use chrome::draw_context_line;
use chrome::draw_status;
use input::draw_input;
use overlays::{
    approval_height, draw_approval, draw_help, draw_menu, draw_picker, draw_plan_approval,
    draw_run_panel, draw_search, draw_trust_modal, plan_approval_height, run_panel_height,
    trust_modal_height,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};
use surface::{draw_empty_state, draw_transcript};

/// Draws one frame: transcript (fills), the run panel (docked below the
/// conversation when a run has been started from the session), status bar,
/// approval panels, input box, popup overlay.
pub(super) fn draw(frame: &mut Frame<'_>, app: &App, status: &StatusView) {
    let input_height = (app.input_rows(frame.area().width as usize) as u16) + 2;
    let run_panel_h = app
        .run_panel
        .as_ref()
        .map(|panel| run_panel_height(panel, frame.area().height))
        .unwrap_or(0);
    let plan_approval_h = app
        .run_panel
        .as_ref()
        .and_then(|panel| panel.plan_approval.as_ref())
        .map(|request| plan_approval_height(&request.view_text, frame.area().width))
        .unwrap_or(0);
    let approval_h = app
        .request
        .pending_approval
        .as_ref()
        .map(|p| approval_height(p.detail.as_deref(), p.grant.as_deref(), frame.area().width))
        .unwrap_or(0);
    // The startup trust modal docks above the input like the approval
    // panels: it opens once after the splash paints, never before it.
    let trust_h = app
        .overlays
        .trust
        .as_ref()
        .map(|prompt| trust_modal_height(prompt, frame.area().width))
        .unwrap_or(0);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),               // context line (row 0)
            Constraint::Min(1),                  // transcript
            Constraint::Length(run_panel_h),     // run panel (0 when none)
            Constraint::Length(1),               // status bar
            Constraint::Length(approval_h),      // approval panel (0 when none)
            Constraint::Length(plan_approval_h), // plan-approval modal (0 when none)
            Constraint::Length(trust_h),         // trust modal (0 when none)
            Constraint::Length(input_height),    // input box
        ])
        .split(frame.area());

    draw_context_line(frame, status, chunks[0]);
    let has_turns = app
        .transcript
        .blocks()
        .iter()
        .any(|b| matches!(b.kind, BlockKind::User | BlockKind::Assistant));
    if has_turns {
        draw_transcript(frame, app, chunks[1]);
    } else {
        // The empty state names the unbound workspace when the status bar's
        // view carries no root: `None` renders the no-root shape beside the
        // no-database guidance, never instead of it.
        draw_empty_state(frame, app, chunks[1], Some(status.workspace_root.is_some()));
    }
    if run_panel_h > 0 {
        draw_run_panel(frame, app, chunks[2]);
    }
    draw_status(frame, app, status, chunks[3]);
    if let Some(pending) = &app.request.pending_approval {
        draw_approval(
            frame,
            &pending.tool,
            pending.detail.as_deref(),
            pending.grant.as_deref(),
            pending.scroll,
            chunks[4],
        );
    }
    if let Some(panel) = &app.run_panel
        && panel.plan_approval.is_some()
    {
        draw_plan_approval(frame, panel, chunks[5]);
    }
    if let Some(prompt) = &app.overlays.trust {
        draw_trust_modal(frame, prompt, chunks[6]);
    }
    draw_input(frame, app, chunks[7]);
    if let Some(menu) = &app.overlays.menu {
        draw_menu(frame, menu, chunks[7]);
    }
    if app.overlays.search.is_some() {
        draw_search(frame, app, frame.area());
    }
    if app.overlays.picker.is_some() {
        draw_picker(frame, app, frame.area());
    }
    if app.overlays.show_help {
        draw_help(frame, frame.area());
    }
}
