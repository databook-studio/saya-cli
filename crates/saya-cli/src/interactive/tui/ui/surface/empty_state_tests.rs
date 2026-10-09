//! The empty state under a short terminal: decoration yields before
//! guidance, in an explicit drop order, and the first screen states AI,
//! project, and database readiness independently — an absent database
//! never reads as what stands between the user and their work.
//! Splash-only tests render `draw_empty_state` directly onto a
//! `TestBackend` at an exact pane height; the composed-screen tests go
//! through the real `ui::draw`.

use super::super::splash::{
    DATABASE_NONE, PROJECT_READY, START_AI, START_CODE_BOUND, START_CODE_UNBOUND, START_HEADER,
    START_OPEN, START_QUERY, START_SETUP,
};
use super::*;
use crate::interactive::tui::startup_readiness::{ReadinessLine, StartupReadiness};
use crate::interactive::tui::ui_snapshot_tests::{
    empty_app, empty_app_with_startup, fixed_status, render_buffer,
};

/// The keyboard hint is the last line of the splash, so a clip from the
/// bottom takes it first — the opposite of the drop order.
const HINT: &str = "/ commands     @ tables     ? help     Ctrl+C quit";

/// The concept line the first screen must name once it has room.
const CONCEPT: &str = "Each request becomes a chapter — activity, answer, and results.";

/// The owl's body glyph marks the art: no splash text contains it.
const ART_GLYPH: char = '\u{2588}';

/// An AI line that is not ready, carrying the credential detail the
/// doctor's `ai` task produces for a cloud provider with no key
/// reference.
fn unready_ai() -> StartupReadiness {
    StartupReadiness {
        ai: ReadinessLine {
            ready: false,
            detail: "no api_key reference configured — requests to openai will be unauthenticated"
                .into(),
        },
    }
}

/// Renders the empty state alone at an exact pane size. `profiles` replaces
/// the fixture's demo list; `workspace_bound` drives the workspace paragraph
/// the way `ui::draw` passes it.
fn splash(width: u16, height: u16, profiles: &[&str], workspace_bound: bool) -> String {
    let mut app = empty_app();
    app.profiles = profiles.iter().map(|name| (*name).to_string()).collect();
    paint(&app, width, height, workspace_bound)
}

/// Renders a prepared `App` at an exact pane size, the way the splash helper
/// does, for tests that need state beyond the profile list.
fn paint(app: &App, width: u16, height: u16, workspace_bound: bool) -> String {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| super::draw_empty_state(frame, app, frame.area(), Some(workspace_bound)))
        .expect("draw completes");
    format!("{}", terminal.backend())
}

#[path = "empty_state_tests/first_screen_copy.rs"]
mod first_screen_copy;
#[path = "empty_state_tests/readiness.rs"]
mod readiness;
#[path = "empty_state_tests/terminal_geometry.rs"]
mod terminal_geometry;
