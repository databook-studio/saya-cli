//! Copy for the home screen: the tiny mascot, the readiness lines the
//! Status block paints, and the short getting-started hints. The copy lives
//! here so tests can assert on it.

use super::super::theme::accent;
use ratatui::{
    style::Style,
    text::{Line, Span},
};

/// The state section's three labels, padded to one width so the three
/// values align down the screen.
pub(in crate::interactive::tui) const AI_LABEL: &str = "AI        ";
pub(in crate::interactive::tui) const PROJECT_LABEL: &str = "Project   ";
pub(in crate::interactive::tui) const DATABASE_LABEL: &str = "Database  ";

/// The AI verdict when the provider is configured; the doctor's provider
/// detail ("ollama, model m") follows it.
pub(in crate::interactive::tui) const AI_READY: &str = "ready — ";

/// The AI verdict when the provider is not configured. The first failing
/// doctor check's detail follows, then the remedy: the AI line carries
/// the in-session instruction, never the database line.
pub(in crate::interactive::tui) const AI_NOT_SET_UP: &str = "not set up — ";
pub(in crate::interactive::tui) const AI_REMEDY: &str = "; try /setup, /provider or /model";

/// The bound-project verdict: with a root bound, coding needs nothing
/// else — the file tools are the difference.
pub(in crate::interactive::tui) const PROJECT_READY: &str = "ready — file tools are available";

/// The no-database verdict: coding does not wait on a database — the
/// line reads as optional and offers the two in-window ways to add one.
pub(in crate::interactive::tui) const DATABASE_NONE: &str =
    "none yet — optional; /connect or `saya demo`";

/// The home screen's left-column introduction and the right column's two
/// section headings.
pub(in crate::interactive::tui) const WELCOME: &str = "Welcome to saya";
pub(in crate::interactive::tui) const GETTING_STARTED: &str = "Getting started";
pub(in crate::interactive::tui) const STATUS: &str = "Status";

/// The status line when no model is configured: the provider and mode are
/// meaningless without one.
pub(in crate::interactive::tui) const AI_NOT_CONFIGURED: &str = "AI not set up";

/// The three getting-started hints, short enough for a stacked 60-column
/// home screen. The AI hint names guided setup when the provider is not
/// ready, and the two commands that change it when it is.
pub(in crate::interactive::tui) const START_SETUP: &str = "/setup — set up AI and a database";
pub(in crate::interactive::tui) const START_AI: &str = "/model or /provider — change AI";

/// The database hint: the two in-window ways to add one, or the direct-SQL
/// door once a profile is configured.
pub(in crate::interactive::tui) const START_DATABASE: &str =
    "/connect or `saya demo` — add a database";
pub(in crate::interactive::tui) const START_SQL: &str = "/sql — run a read-only query";

/// The coding hint: the mode switch and native jobs. Shift+Tab lives here
/// (never in a footer) so it stays discoverable in both workspace states.
pub(in crate::interactive::tui) const START_CODE_BOUND: &str =
    "Shift+Tab — change mode; `saya start` runs a job";
pub(in crate::interactive::tui) const START_CODE_UNBOUND: &str =
    "Shift+Tab — change mode; `saya start` — run a job";

/// The headline shown when no workspace root is bound, beside the state
/// section's own project line when the root is absent: the session names
/// the unbound shape rather than staying silent about it.
pub(crate) const NO_WORKSPACE_LINES: [&str; 2] = [
    "No workspace is bound: file tools are unavailable;",
    "launch inside a git worktree or pass `--workspace <dir>`.",
];

/// The tiny mascot: a right-facing kangaroo profile — two tall ears, a head
/// with one eye gap and a snout, a body, and a tail sweeping left to the
/// ground. Four rows by ten columns, one size only.
///
/// Only block glyphs and spaces are used, so it reads as a shape in
/// monochrome and in 256-colour terminals alike; [`mascot_lines`] paints it
/// in the theme's iris accent. Keep every row the same width: the left
/// column centres the figure, and a ragged row would skew it.
pub(in crate::interactive::tui) const MASCOT: [&str; 4] = [
    "    \u{2588}\u{2588} \u{2588}\u{2588} ",
    "  \u{2588}\u{2588}\u{2588} \u{2588}\u{2588}\u{2588}\u{2590}",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ",
    "\u{2584}\u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2580}  ",
];

/// Paints the mascot rows in the accent colour.
pub(super) fn mascot_lines() -> Vec<Line<'static>> {
    MASCOT
        .iter()
        .map(|row| Line::from(Span::styled(*row, Style::default().fg(accent()))))
        .collect()
}

/// The left column's dim status line: model, provider, and the permission
/// mode Shift+Tab moves. A session with no model says so instead of
/// naming a provider it cannot use.
pub(super) fn status_text(model: &str, provider: &str, mode: &str) -> String {
    if model.trim().is_empty() {
        return AI_NOT_CONFIGURED.to_string();
    }
    format!("{model} · {provider} · {mode}")
}

#[cfg(test)]
#[path = "splash_tests.rs"]
mod tests;
