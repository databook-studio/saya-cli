//! Resolves the `[ui] theme` choice for a real terminal session.
//!
//! `Auto` asks the terminal for its background over OSC 10/11 before
//! falling back to [`resolve_theme`]'s existing `COLORFGBG` heuristic.
//! [`resolve_startup_theme`] is the entry point `session/run.rs` calls;
//! [`detect_theme`] is its pure core, tested with an injected probe.

use super::palette::{Theme, resolve_theme};
use saya_config::ThemeChoice;
use std::time::Duration;

/// How long `Auto` waits for the terminal to answer the background query.
/// Long enough that a local terminal's reply lands well inside it (native
/// terminals answer in single-digit milliseconds); short enough that a
/// laggy SSH hop falls back to `COLORFGBG` instead of stalling startup.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_millis(150);

/// The pure decision for `Auto`: `probe` answers once and, when it does,
/// wins over `COLORFGBG`. `None` (unsupported terminal, timeout, error)
/// falls back to the existing `COLORFGBG` heuristic. `Dark`/`Light` never
/// call `probe` — they force a palette and ignore the terminal entirely.
fn detect_theme(
    choice: ThemeChoice,
    probe: impl FnOnce() -> Option<Theme>,
    colorfgbg: Option<&str>,
) -> Theme {
    match choice {
        ThemeChoice::Dark | ThemeChoice::Light => resolve_theme(choice, colorfgbg),
        ThemeChoice::Auto => probe().unwrap_or_else(|| resolve_theme(choice, colorfgbg)),
    }
}

/// The startup entry point. Gates the terminal query on colour being on and
/// stdout being a real terminal: a redirected pipe or a colour-off session
/// has nothing to answer a query, so `probe` is never invoked in that case
/// and resolution falls straight to [`resolve_theme`].
pub(crate) fn resolve_startup_theme(
    choice: ThemeChoice,
    color_enabled: bool,
    is_terminal: bool,
    probe: impl FnOnce() -> Option<Theme>,
    colorfgbg: Option<&str>,
) -> Theme {
    if color_enabled && is_terminal {
        detect_theme(choice, probe, colorfgbg)
    } else {
        resolve_theme(choice, colorfgbg)
    }
}

/// The real probe: asks the terminal for its background over OSC 10/11 via
/// `terminal-colorsaurus`, which restores the terminal's mode itself even on
/// error or panic. `None` on any unsupported terminal, timeout, or error —
/// [`detect_theme`] falls back to `COLORFGBG` in that case.
pub(crate) fn probe_terminal_theme() -> Option<Theme> {
    // `QueryOptions` is `#[non_exhaustive]`, so the default is built first
    // and the one field we care about is set on it.
    let mut options = terminal_colorsaurus::QueryOptions::default();
    options.timeout = PROBE_TIMEOUT;
    match terminal_colorsaurus::theme_mode(options) {
        Ok(terminal_colorsaurus::ThemeMode::Dark) => Some(Theme::Dark),
        Ok(terminal_colorsaurus::ThemeMode::Light) => Some(Theme::Light),
        Err(_) => None,
    }
}

#[cfg(test)]
#[path = "detect_tests.rs"]
mod tests;
