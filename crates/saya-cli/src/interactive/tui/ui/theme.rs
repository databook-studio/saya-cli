//! Colour palette and shared style helpers for the TUI.
//!
//! The palette honours the resolved `output.color` setting (and the standard
//! `NO_COLOR` convention under `Auto`): every entry routes through [`c`], so
//! disabling colour degrades the whole UI to the terminal default style.

use crate::interactive::tui::transcript::BlockKind;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
};
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

/// Turns the whole TUI palette on or off. Called once at session start from
/// the resolved configuration.
pub(crate) fn set_color_enabled(enabled: bool) {
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Decides colour support from the config choice, terminal type, and env.
fn decide(choice: saya_config::ColorChoice, is_tty: bool, no_color_env: bool) -> bool {
    match choice {
        saya_config::ColorChoice::Always => true,
        saya_config::ColorChoice::Never => false,
        saya_config::ColorChoice::Auto => is_tty && !no_color_env,
    }
}

/// Public shim so the TUI entry point can apply [`decide`] without leaking
/// the config type into every style helper's caller.
pub(crate) fn decide_public(
    choice: saya_config::ColorChoice,
    is_tty: bool,
    no_color_env: bool,
) -> bool {
    decide(choice, is_tty, no_color_env)
}

/// Maps a palette entry through the colour switch.
fn c(color: Color) -> Color {
    if COLOR_ENABLED.load(Ordering::Relaxed) {
        color
    } else {
        Color::Reset
    }
}

/// saya's signature accent (iris violet): brand, assistant, focus, borders.
pub(super) fn accent() -> Color {
    c(Color::Rgb(157, 139, 245))
}
/// User turns — a cool secondary so the accent stays saya's.
pub(super) fn user_color() -> Color {
    c(Color::Rgb(106, 155, 204))
}
/// Secondary/de-emphasised text: tool + system lines, hints, provider/model.
pub(super) fn secondary() -> Color {
    c(Color::Rgb(168, 162, 154))
}
/// Status: success / safe (read-only approval, privacy on, passing checks).
pub(super) fn success() -> Color {
    c(Color::Rgb(127, 174, 107))
}
/// Status: caution (ask approval, approval-panel border).
pub(super) fn warning() -> Color {
    c(Color::Rgb(224, 164, 88))
}
/// Status: error / danger (failures, never approval).
pub(super) fn danger() -> Color {
    c(Color::Rgb(229, 105, 95))
}
/// Status-bar / badge background (faint iris-tinted dark).
pub(super) fn status_bg() -> Color {
    c(Color::Rgb(30, 28, 36))
}
/// Inline `code` in assistant answers.
pub(super) fn code_color() -> Color {
    c(Color::Rgb(127, 181, 214))
}

/// Maps a transcript block kind to its display style.
pub(super) fn kind_style(kind: BlockKind) -> Style {
    match kind {
        BlockKind::User => Style::default()
            .fg(user_color())
            .add_modifier(Modifier::BOLD),
        BlockKind::Assistant => Style::default().fg(c(Color::White)),
        BlockKind::System => Style::default()
            .fg(secondary())
            .add_modifier(Modifier::ITALIC),
        BlockKind::Error => Style::default().fg(danger()).add_modifier(Modifier::BOLD),
        BlockKind::Tool => Style::default().fg(secondary()),
        // The model's chain-of-thought restates database contents in prose and
        // is often longer than the answer, so it stays visually subordinate:
        // dimmed secondary, never mistakable for the answer.
        BlockKind::Thinking => Style::default().fg(secondary()).add_modifier(Modifier::DIM),
    }
}

/// Returns the rail color style for a given transcript block kind.
pub(super) fn rail_style(kind: BlockKind) -> Style {
    match kind {
        BlockKind::User => Style::default()
            .fg(user_color())
            .add_modifier(Modifier::BOLD),
        BlockKind::Assistant => Style::default().fg(accent()).add_modifier(Modifier::BOLD),
        BlockKind::Tool | BlockKind::System => Style::default().fg(secondary()),
        BlockKind::Error => Style::default().fg(danger()).add_modifier(Modifier::BOLD),
        BlockKind::Thinking => Style::default().fg(secondary()),
    }
}

/// Centers a `width`×`height` rect within `screen`.
pub(super) fn centered(screen: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: screen.x + (screen.width.saturating_sub(width)) / 2,
        y: screen.y + (screen.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::{COLOR_ENABLED, decide};
    use saya_config::ColorChoice;
    use std::sync::atomic::Ordering;

    #[test]
    fn color_decision_follows_choice_env_and_tty() {
        assert!(decide(ColorChoice::Always, false, true));
        assert!(!decide(ColorChoice::Never, true, false));
        assert!(decide(ColorChoice::Auto, true, false));
        assert!(!decide(ColorChoice::Auto, true, true), "NO_COLOR disables");
        assert!(!decide(ColorChoice::Auto, false, false), "pipes stay plain");

        // The global switch starts enabled and still works after a toggle
        // round-trip (single-threaded test binaries aside, this only proves
        // reachability).
        let before = COLOR_ENABLED.load(Ordering::Relaxed);
        COLOR_ENABLED.store(!before, Ordering::Relaxed);
        assert_eq!(COLOR_ENABLED.load(Ordering::Relaxed), !before);
        COLOR_ENABLED.store(before, Ordering::Relaxed);
    }
}
