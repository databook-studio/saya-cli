//! Colour palette and shared style helpers for the TUI.
//!
//! The palette honours the resolved `output.color` setting (and the standard
//! `NO_COLOR` convention under `Auto`): every entry routes through [`c`], so
//! disabling colour degrades the whole UI to the terminal default style.
//!
//! Two palettes — dark and light — live behind the same accessors. The active
//! one is chosen once at session start from the `[ui] theme` choice and held in
//! a process-global, so a call site never learns which palette is in effect: it
//! calls [`accent`] or [`secondary`], and the right colour comes out. A
//! hard-coded colour anywhere outside this module is the bug that centralisation
//! exists to prevent.

use crate::interactive::tui::transcript::BlockKind;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
};
use saya_config::ThemeChoice;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub(super) static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);
static ACTIVE_THEME: AtomicU8 = AtomicU8::new(Theme::DARK_TAG);

/// Turns the whole TUI palette on or off. Called once at session start from
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

/// Public shim so the TUI entry point can apply [`decide`] without leaking
/// the config type into every style helper's caller.
pub(crate) fn decide_public(
    choice: saya_config::ColorChoice,
    is_tty: bool,
    no_color_env: bool,
) -> bool {
    decide(choice, is_tty, no_color_env)
}

/// The concrete palette the TUI paints with. Resolved from `ThemeChoice` once
/// at startup and stored as a tag, since `AtomicU8` is the smallest primitive
/// that holds a two-variant enum without a lock.
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
pub(crate) fn resolve_theme(choice: ThemeChoice, colorfgbg: Option<&str>) -> Theme {
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

/// Selects the dark or light variant of a palette entry and routes it through
/// the colour switch, so a single accessor is the whole story at every call
/// site: theme choice and colour-disable both stay invisible to callers.
fn pick(dark: Color, light: Color) -> Color {
    c(match active_theme() {
        Theme::Dark => dark,
        Theme::Light => light,
    })
}

/// saya's signature accent (iris violet): brand, assistant, focus, borders.
pub(in crate::interactive::tui) fn accent() -> Color {
    pick(Color::Rgb(157, 139, 245), Color::Rgb(109, 78, 200))
}
/// User turns — a cool secondary so the accent stays saya's.
pub(in crate::interactive::tui) fn user_color() -> Color {
    pick(Color::Rgb(106, 155, 204), Color::Rgb(44, 108, 176))
}
/// Secondary/de-emphasised text: tool + system lines, hints, provider/model.
pub(in crate::interactive::tui) fn secondary() -> Color {
    pick(Color::Rgb(168, 162, 154), Color::Rgb(107, 102, 96))
}
/// Status: success / safe (read-only approval, `sharing:off` — data stays
/// local, passing checks).
pub(in crate::interactive::tui) fn success() -> Color {
    pick(Color::Rgb(127, 174, 107), Color::Rgb(61, 122, 53))
}
/// Status: caution (ask approval, approval-panel border, `sharing:on` — row
/// values are being sent to the provider).
pub(in crate::interactive::tui) fn warning() -> Color {
    pick(Color::Rgb(224, 164, 88), Color::Rgb(154, 93, 18))
}
/// Status: error / danger (failures, never approval).
pub(in crate::interactive::tui) fn danger() -> Color {
    pick(Color::Rgb(229, 105, 95), Color::Rgb(181, 55, 44))
}
/// Status-bar / badge background (faint iris-tinted dark, or a light tint).
pub(in crate::interactive::tui) fn status_bg() -> Color {
    pick(Color::Rgb(30, 28, 36), Color::Rgb(236, 233, 245))
}
/// Inline `code` in assistant answers.
pub(in crate::interactive::tui) fn code_color() -> Color {
    pick(Color::Rgb(127, 181, 214), Color::Rgb(46, 109, 158))
}
/// Primary foreground for body text on the terminal's default ground. The
/// dark entry is the terminal's own text colour (`Reset`), never a fixed
/// white: a wrong dark/light guess can then never paint the answer
/// invisible on a light terminal. The light entry keeps its near-black,
/// which is deliberately painted (the light palette is only ever chosen
/// when the terminal is known to be light).
pub(in crate::interactive::tui) fn foreground() -> Color {
    pick(Color::Reset, Color::Rgb(40, 38, 42))
}
/// Text colour that reads against the accent when the accent is the background
/// (selection badges, highlighted rows): black on the light iris, white on the
/// deeper light-theme iris.
pub(in crate::interactive::tui) fn on_accent() -> Color {
    pick(Color::Black, Color::White)
}

/// Maps a transcript block kind to its display style.
pub(in crate::interactive::tui) fn kind_style(kind: BlockKind) -> Style {
    match kind {
        BlockKind::User => Style::default()
            .fg(user_color())
            .add_modifier(Modifier::BOLD),
        BlockKind::Assistant => Style::default().fg(foreground()),
        BlockKind::System => Style::default()
            .fg(secondary())
            .add_modifier(Modifier::ITALIC),
        BlockKind::Error => Style::default().fg(danger()).add_modifier(Modifier::BOLD),
        BlockKind::Tool => Style::default().fg(secondary()),
        // A result table is grid text, so it reads like inline code: aligned
        // and monospaced-feeling, distinct from prose tool lines.
        BlockKind::Table => Style::default().fg(code_color()),
        // The model's chain-of-thought restates database contents in prose and
        // is often longer than the answer, so it stays visually subordinate:
        // dimmed secondary, never mistakable for the answer.
        BlockKind::Thinking => Style::default().fg(secondary()).add_modifier(Modifier::DIM),
    }
}

/// The label-row style: plain, uncoloured, unmodified uppercase words.
/// Labels are the only thing marking a turn now, so they must read without
/// colour — and every other row keeps its `kind_style`, untouched.
pub(in crate::interactive::tui) fn label_style() -> Style {
    Style::default()
}

/// Centers a `width`×`height` rect within `screen`.
pub(in crate::interactive::tui) fn centered(screen: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: screen.x + (screen.width.saturating_sub(width)) / 2,
        y: screen.y + (screen.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}
