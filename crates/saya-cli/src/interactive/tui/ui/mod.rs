//! Rendering for the TUI: transcript, status bar, input box, and overlays.
//! Kept separate from the event loop so styling can evolve on its own.

mod input_box;
mod markdown;
mod overlays;
mod panels;
mod status;
pub(crate) mod theme;

use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;
use input_box::draw_input;
use overlays::{draw_help, draw_menu, draw_picker, draw_search};
use panels::{approval_height, draw_approval, draw_empty_state, draw_transcript};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};
use status::draw_status;

/// Draws one frame: transcript (fills), status bar, approval panel (when pending), input box, popup overlay.
pub(super) fn draw(frame: &mut Frame<'_>, app: &App, status: &StatusView) {
    let input_height = (app.input_rows() as u16) + 2;
    let approval_h = app
        .request
        .pending_approval
        .as_ref()
        .map(|p| approval_height(p.detail.as_deref(), frame.area().width))
        .unwrap_or(0);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),               // transcript
            Constraint::Length(1),            // status bar
            Constraint::Length(approval_h),   // approval panel (0 when none)
            Constraint::Length(input_height), // input box
        ])
        .split(frame.area());

    let has_turns = app
        .transcript
        .blocks()
        .iter()
        .any(|b| matches!(b.kind, BlockKind::User | BlockKind::Assistant));
    if has_turns {
        draw_transcript(frame, app, chunks[0]);
    } else {
        draw_empty_state(frame, app, chunks[0]);
    }
    draw_status(frame, app, status, chunks[1]);
    if let Some(pending) = &app.request.pending_approval {
        draw_approval(frame, &pending.tool, pending.detail.as_deref(), chunks[2]);
    }
    draw_input(frame, app, chunks[3]);
    if let Some(menu) = &app.overlays.menu {
        draw_menu(frame, menu, chunks[3]);
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
