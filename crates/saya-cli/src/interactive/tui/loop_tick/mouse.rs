//! Reconciling terminal mouse capture with selection mode.

use super::super::transcript::BlockKind;
use super::super::types::App;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use std::io::Stdout;

pub(crate) struct MouseCapture {
    pub(crate) captured: bool,
    pub(crate) error_reported: bool,
}

/// Reconciles the terminal's mouse capture with selection mode: releasing
/// capture lets the terminal drag-select and copy; re-grabbing it restores
/// wheel scrolling.
pub(crate) fn tick_mouse_capture(
    app: &mut App,
    backend: &mut CrosstermBackend<Stdout>,
    state: &mut MouseCapture,
) {
    let want_capture = !app.overlays.selection_mode;
    if want_capture != state.captured {
        let result = if want_capture {
            execute!(backend, EnableMouseCapture)
        } else {
            execute!(backend, DisableMouseCapture)
        };
        match result {
            Ok(()) => {
                state.captured = want_capture;
                state.error_reported = false;
            }
            Err(error) if !state.error_reported => {
                app.transcript.push(
                    BlockKind::Error,
                    format!("Could not update mouse capture: {error}"),
                );
                state.error_reported = true;
            }
            Err(_) => {}
        }
    }
}
