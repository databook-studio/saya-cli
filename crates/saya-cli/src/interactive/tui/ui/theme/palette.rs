//! The ground a launch paints with, and the colours the answer body reads.
//!
//! Two palettes — dark and light — are chosen once at session start from the
//! `[ui] theme` choice (and `--theme`), and the colour switch from
//! `output.color` and `NO_COLOR`. The new surface's token table (`front/tui's
//! `theme`) reads the hues directly; the kept markdown renderer reads them
//! through the accessors here, so an answer's inline code and accent obey the
//! same choice as everything the new surface paints.

use super::hue;
use ratatui::style::Color;
use saya_config::ThemeChoice;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Whether colour is painted at all: off in a redirected or `NO_COLOR` session.
pub(super) static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);
static ACTIVE_THEME: AtomicU8 = AtomicU8::new(Theme::DARK_TAG);

/// Turns the answer body's palette on or off. Called once at session start from
/// the resolved configuration.
pub(crate) fn set_color_enabled(enabled: bool) {
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Decides colour support from the config choice, terminal type, and env.
pub(super) fn decide(choice: saya_config::ColorChoice, is_tty: bool, no_color_env: bool) -> bool {
    match choice {
        saya_config::ColorChoice::Always => true,
        saya_config::ColorChoice::Never => false,
        saya_config::ColorChoice::Auto => is_tty && !no_color_env,
    }
}

/// Public shim so the launch can apply [`decide`] without leaking the config
/// type into every caller.
pub(crate) fn decide_public(
    choice: saya_config::ColorChoice,
    is_tty: bool,
    no_color_env: bool,
) -> bool {
    decide(choice, is_tty, no_color_env)
}

/// The concrete palette a session paints with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Theme {
    Dark,
    Light,
}

impl Theme {
    const DARK_TAG: u8 = 0;
    const LIGHT_TAG: u8 = 1;

    fn to_tag(self) -> u8 {
        match self {
            Self::Dark => Self::DARK_TAG,
            Self::Light => Self::LIGHT_TAG,
        }
    }

    fn from_tag(tag: u8) -> Self {
        match tag {
            Self::LIGHT_TAG => Self::Light,
            _ => Self::Dark,
        }
    }
}

/// Sets the active palette. Called once at session start from the resolved
/// `[ui] theme` choice; every accessor reads the result.
pub(crate) fn set_theme(theme: Theme) {
    ACTIVE_THEME.store(theme.to_tag(), Ordering::Relaxed);
}

/// Resolves the `[ui] theme` choice to a concrete palette.
///
/// `Auto` honours `COLORFGBG` when the terminal publishes it: the value is
/// `fg;bg` with ANSI colour indices, and a background of 7–15 is a light
/// terminal, so that selects the light palette. A background of 0–6, an
/// unparseable value, or `COLORFGBG` absent all fall back to dark — the common
/// case — rather than silently guessing a user into the wrong theme. `Dark` and
/// `Light` force a palette and ignore the environment.
pub(super) fn resolve_theme(choice: ThemeChoice, colorfgbg: Option<&str>) -> Theme {
    match choice {
        ThemeChoice::Dark => Theme::Dark,
        ThemeChoice::Light => Theme::Light,
        ThemeChoice::Auto => match colorfgbg.and_then(colorfgbg_background) {
            Some(bg) if bg >= 7 => Theme::Light,
            _ => Theme::Dark,
        },
    }
}

/// Extracts the background index (the second field) from a `COLORFGBG` value.
/// Non-numeric fields such as `default` yield `None`, so an ambiguous terminal
/// reports nothing and `Auto` keeps the dark fallback.
fn colorfgbg_background(value: &str) -> Option<u32> {
    value
        .split(';')
        .nth(1)
        .and_then(|field| field.trim().parse::<u32>().ok())
}

fn active_theme() -> Theme {
    Theme::from_tag(ACTIVE_THEME.load(Ordering::Relaxed))
}

/// Maps a palette entry through the colour switch.
fn c(color: Color) -> Color {
    if COLOR_ENABLED.load(Ordering::Relaxed) {
        color
    } else {
        Color::Reset
    }
}

/// Selects the dark or light variant of an entry and routes it through the
/// colour switch, so a call site never learns which palette is in force.
fn pick(dark: Color, light: Color) -> Color {
    c(match active_theme() {
        Theme::Dark => dark,
        Theme::Light => light,
    })
}

/// saya's signature accent (iris violet): brand, assistant, emphasis.
pub(in crate::interactive::tui) fn accent() -> Color {
    pick(hue::ACCENT_DARK, hue::ACCENT_LIGHT)
}

/// Secondary/de-emphasised text: tool + system lines, hints.
pub(in crate::interactive::tui) fn secondary() -> Color {
    pick(hue::DIM_DARK, hue::DIM_LIGHT)
}

/// Inline `code` in assistant answers.
pub(in crate::interactive::tui) fn code_color() -> Color {
    pick(hue::CODE_DARK, hue::CODE_LIGHT)
}

/// Primary foreground for body text on the terminal's default ground. The dark
/// entry is the terminal's own text colour (`Reset`), never a fixed white: a
/// wrong dark/light guess can then never paint the answer invisible on a light
/// terminal. The light entry keeps its near-black, which is deliberately
/// painted (the light palette is only ever chosen when the terminal is known
/// to be light).
pub(in crate::interactive::tui) fn foreground() -> Color {
    pick(Color::Reset, hue::FOREGROUND_LIGHT)
}
