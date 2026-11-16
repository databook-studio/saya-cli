//! Copy and line construction for the home screen: the welcome column, the
//! Getting-started hints, and the Status block. Kept apart from the layout
//! so each file stays one concern.

use super::super::super::theme::{accent, foreground, secondary, warning};
use super::super::splash::{
    GETTING_STARTED, NO_WORKSPACE_LINES, START_AI, START_CODE_BOUND, START_CODE_UNBOUND,
    START_DATABASE, START_SETUP, START_SQL, STATUS, WELCOME, mascot_lines, status_text,
};
use super::state;
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::types::App;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// The section heading: accent, bold.
fn heading(text: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::default().fg(accent()).add_modifier(Modifier::BOLD),
    ))
}

/// One getting-started hint: quiet, so the heading carries the emphasis.
fn hint(text: &'static str) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(secondary())))
}

/// A stated absence reads as a caveat: warning colour, bold.
fn missing(text: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::default().fg(warning()).add_modifier(Modifier::BOLD),
    ))
}

/// The left column's welcome line.
fn welcome() -> Line<'static> {
    Line::from(Span::styled(
        WELCOME,
        Style::default()
            .fg(foreground())
            .add_modifier(Modifier::BOLD),
    ))
}

/// The left column's dim status line: model, provider, permission mode.
fn status_line(status: &StatusView, provider: &str, mode: &str) -> Line<'static> {
    Line::from(Span::styled(
        status_text(&status.model, provider, mode),
        Style::default().fg(secondary()),
    ))
}

/// The permission-mode word Shift+Tab moves. A consent posture with no mode
/// of its own (`read-only`, `never`) keeps the approval word the status bar
/// already shows; an empty one falls back to the default.
pub(super) fn mode_text(status: &StatusView) -> &str {
    match status.permission_mode {
        Some(mode) => crate::interactive::session::permissions::mode::name(mode),
        None if !status.approval_mode.is_empty() => status.approval_mode.as_str(),
        None => "ask",
    }
}

/// The full left column: welcome, mascot, status line.
pub(super) fn left_lines(status: &StatusView, provider: &str, mode: &str) -> Vec<Line<'static>> {
    let mut lines = vec![welcome(), Line::from("")];
    lines.extend(mascot_lines());
    lines.push(Line::from(""));
    lines.push(status_line(status, provider, mode));
    lines
}

/// The left column without its mascot: the first thing a short terminal
/// drops.
pub(super) fn left_lines_without_mascot(
    status: &StatusView,
    provider: &str,
    mode: &str,
) -> Vec<Line<'static>> {
    vec![
        welcome(),
        Line::from(""),
        status_line(status, provider, mode),
    ]
}

/// A horizontal rule spanning the column, drawn between the two right-hand
/// sections.
pub(super) fn rule_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(1)),
        Style::default().fg(secondary()),
    ))
}

/// The three short ways in, chosen by state: setup or change AI, add a
/// database or query it, and the coding door for the workspace binding.
fn getting_started_lines(
    app: &App,
    workspace_bound: Option<bool>,
    width: usize,
) -> Vec<Line<'static>> {
    let setup = if app.startup.ai.ready {
        START_AI
    } else {
        START_SETUP
    };
    let database = if app.profiles.is_empty() {
        START_DATABASE
    } else {
        START_SQL
    };
    let code = if workspace_bound == Some(true) {
        START_CODE_BOUND
    } else {
        START_CODE_UNBOUND
    };
    vec![
        heading(GETTING_STARTED),
        hint(setup),
        hint(database),
        hint(code),
        Line::from(""),
        rule_line(width),
        Line::from(""),
    ]
}

/// The unbound-workspace notice, beside — never instead of — the Status
/// block: the absences are orthogonal and both must read.
fn workspace_notice() -> Vec<Line<'static>> {
    NO_WORKSPACE_LINES
        .iter()
        .map(|text| missing(text))
        .collect()
}

/// The Status block: its heading, the three readiness lines, and the
/// unbound-workspace notice when no root binds.
pub(super) fn status_block(app: &App, workspace_bound: Option<bool>) -> Vec<Line<'static>> {
    let mut lines = vec![heading(STATUS)];
    lines.extend(state::status_lines(app, workspace_bound));
    if workspace_bound == Some(false) {
        lines.push(Line::from(""));
        lines.extend(workspace_notice());
    }
    lines
}

/// The full right column: Getting started, a rule, then Status.
pub(super) fn right_lines(
    app: &App,
    workspace_bound: Option<bool>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = getting_started_lines(app, workspace_bound, width);
    lines.extend(status_block(app, workspace_bound));
    lines
}
