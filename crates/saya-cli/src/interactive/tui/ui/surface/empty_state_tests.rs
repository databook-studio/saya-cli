//! The home screen: one rounded box with a Welcome column and a
//! Getting-started/Status column, a footer below it, and a drop order that
//! keeps the stated state last. Splash-only tests render `draw_empty_state`
//! directly onto a `TestBackend` at an exact pane size; the composed-screen
//! tests go through the real `ui::draw`.

use super::super::splash::{
    AI_NOT_CONFIGURED, DATABASE_NONE, GETTING_STARTED, NO_WORKSPACE_LINES, PROJECT_READY, START_AI,
    START_CODE_BOUND, START_CODE_UNBOUND, START_DATABASE, START_SETUP, START_SQL, STATUS, WELCOME,
};
use super::*;
use crate::interactive::tui::startup_readiness::{ReadinessLine, StartupReadiness};
use crate::interactive::tui::ui_snapshot_tests::{
    empty_app, empty_app_with_startup, fixed_status, render_buffer,
};

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

/// Renders the home screen alone at an exact pane size. `profiles` replaces
/// the fixture's demo list; `workspace_bound` drives the workspace notice
/// the way `ui::draw` passes it.
fn splash(width: u16, height: u16, profiles: &[&str], workspace_bound: bool) -> String {
    let mut app = empty_app();
    app.profiles = profiles.iter().map(|name| (*name).to_string()).collect();
    paint(&app, width, height, workspace_bound)
}

/// Renders a prepared `App` with the fixture's status view.
fn paint(app: &App, width: u16, height: u16, workspace_bound: bool) -> String {
    paint_with(app, &fixed_status(), width, height, workspace_bound)
}

/// Renders a prepared `App` and status at an exact pane size, the way the
/// splash helper does, for tests that need state beyond the profile list.
fn paint_with(
    app: &App,
    status: &crate::interactive::session_prompt::StatusView,
    width: u16,
    height: u16,
    workspace_bound: bool,
) -> String {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| {
            super::draw_empty_state(frame, app, status, frame.area(), Some(workspace_bound))
        })
        .expect("draw completes");
    format!("{}", terminal.backend())
}

#[path = "empty_state_tests/first_screen_copy.rs"]
mod first_screen_copy;
#[path = "empty_state_tests/readiness.rs"]
mod readiness;
#[path = "empty_state_tests/terminal_geometry.rs"]
mod terminal_geometry;
