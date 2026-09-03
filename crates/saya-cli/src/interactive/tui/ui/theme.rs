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

static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);
static ACTIVE_THEME: AtomicU8 = AtomicU8::new(Theme::DARK_TAG);

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
pub(super) fn accent() -> Color {
    pick(Color::Rgb(157, 139, 245), Color::Rgb(109, 78, 200))
}
/// User turns — a cool secondary so the accent stays saya's.
pub(super) fn user_color() -> Color {
    pick(Color::Rgb(106, 155, 204), Color::Rgb(44, 108, 176))
}
/// Secondary/de-emphasised text: tool + system lines, hints, provider/model.
pub(super) fn secondary() -> Color {
    pick(Color::Rgb(168, 162, 154), Color::Rgb(107, 102, 96))
}
/// Status: success / safe (read-only approval, privacy on, passing checks).
pub(super) fn success() -> Color {
    pick(Color::Rgb(127, 174, 107), Color::Rgb(61, 122, 53))
}
/// Status: caution (ask approval, approval-panel border).
pub(super) fn warning() -> Color {
    pick(Color::Rgb(224, 164, 88), Color::Rgb(154, 93, 18))
}
/// Status: error / danger (failures, never approval).
pub(super) fn danger() -> Color {
    pick(Color::Rgb(229, 105, 95), Color::Rgb(181, 55, 44))
}
/// Status-bar / badge background (faint iris-tinted dark, or a light tint).
pub(super) fn status_bg() -> Color {
    pick(Color::Rgb(30, 28, 36), Color::Rgb(236, 233, 245))
}
/// Inline `code` in assistant answers.
pub(super) fn code_color() -> Color {
    pick(Color::Rgb(127, 181, 214), Color::Rgb(46, 109, 158))
}
/// Primary foreground for body text on the terminal's default ground: the
/// terminal's light on a dark theme, a near-black on a light theme.
pub(super) fn foreground() -> Color {
    pick(Color::White, Color::Rgb(40, 38, 42))
}
/// Text colour that reads against the accent when the accent is the background
/// (selection badges, highlighted rows): black on the light iris, white on the
/// deeper light-theme iris.
pub(super) fn on_accent() -> Color {
    pick(Color::Black, Color::White)
}

/// Maps a transcript block kind to its display style.
pub(super) fn kind_style(kind: BlockKind) -> Style {
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
struct PaletteSnapshot {
    accent: Color,
    user_color: Color,
    secondary: Color,
    success: Color,
    warning: Color,
    danger: Color,
    status_bg: Color,
    code_color: Color,
    foreground: Color,
    on_accent: Color,
}

/// Gathers every palette accessor into one snapshot so the pin and contrast
/// tests cover the whole surface from one place — adding a new accessor means
/// adding it here, and the routing test then catches a colour that bypasses
/// the central switch.
#[cfg(test)]
fn theme_accessors() -> PaletteSnapshot {
    PaletteSnapshot {
        accent: accent(),
        user_color: user_color(),
        secondary: secondary(),
        success: success(),
        warning: warning(),
        danger: danger(),
        status_bg: status_bg(),
        code_color: code_color(),
        foreground: foreground(),
        on_accent: on_accent(),
    }
}

#[cfg(test)]
mod tests {
    use super::{COLOR_ENABLED, Theme, decide, resolve_theme, set_theme, theme_accessors};
    use ratatui::style::Color;
    use saya_config::{ColorChoice, ThemeChoice};
    use std::sync::{Mutex, atomic::Ordering};

    /// The palette lives in process-globals (`COLOR_ENABLED`, `ACTIVE_THEME`),
    /// so tests that read or write them must run one at a time; without this
    /// guard a parallel test that disables colour makes every accessor return
    /// `Reset` underneath the pin tests.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Snapshot the dark palette. A change here must be deliberate, and the
    /// contrast test below still has to pass for the new values.
    #[test]
    fn dark_palette_is_pinned() {
        let _guard = lock();
        set_theme(Theme::Dark);
        let palette = theme_accessors();
        assert_eq!(palette.accent, Color::Rgb(157, 139, 245));
        assert_eq!(palette.user_color, Color::Rgb(106, 155, 204));
        assert_eq!(palette.secondary, Color::Rgb(168, 162, 154));
        assert_eq!(palette.success, Color::Rgb(127, 174, 107));
        assert_eq!(palette.warning, Color::Rgb(224, 164, 88));
        assert_eq!(palette.danger, Color::Rgb(229, 105, 95));
        assert_eq!(palette.status_bg, Color::Rgb(30, 28, 36));
        assert_eq!(palette.code_color, Color::Rgb(127, 181, 214));
        assert_eq!(palette.foreground, Color::White);
        assert_eq!(palette.on_accent, Color::Black);
    }

    /// Snapshot the light palette. The foregrounds are all deeper than their
    /// dark counterparts so they read on a light ground.
    #[test]
    fn light_palette_is_pinned() {
        let _guard = lock();
        set_theme(Theme::Light);
        let palette = theme_accessors();
        assert_eq!(palette.accent, Color::Rgb(109, 78, 200));
        assert_eq!(palette.user_color, Color::Rgb(44, 108, 176));
        assert_eq!(palette.secondary, Color::Rgb(107, 102, 96));
        assert_eq!(palette.success, Color::Rgb(61, 122, 53));
        assert_eq!(palette.warning, Color::Rgb(154, 93, 18));
        assert_eq!(palette.danger, Color::Rgb(181, 55, 44));
        assert_eq!(palette.status_bg, Color::Rgb(236, 233, 245));
        assert_eq!(palette.code_color, Color::Rgb(46, 109, 158));
        assert_eq!(palette.foreground, Color::Rgb(40, 38, 42));
        assert_eq!(palette.on_accent, Color::White);
    }

    /// Disabling colour must collapse every accessor to `Reset` — this is the
    /// pin that every colour still routes through the central switch rather
    /// than being hard-coded at a call site. A new accessor that forgets `c`
    /// shows up here as a non-`Reset` return.
    #[test]
    fn every_accessor_routes_through_the_colour_switch() {
        let _guard = lock();
        set_theme(Theme::Light);
        let before = COLOR_ENABLED.load(Ordering::Relaxed);
        COLOR_ENABLED.store(false, Ordering::Relaxed);
        let palette = theme_accessors();
        assert_eq!(palette.accent, Color::Reset);
        assert_eq!(palette.user_color, Color::Reset);
        assert_eq!(palette.secondary, Color::Reset);
        assert_eq!(palette.success, Color::Reset);
        assert_eq!(palette.warning, Color::Reset);
        assert_eq!(palette.danger, Color::Reset);
        assert_eq!(palette.status_bg, Color::Reset);
        assert_eq!(palette.code_color, Color::Reset);
        assert_eq!(palette.foreground, Color::Reset);
        assert_eq!(palette.on_accent, Color::Reset);
        COLOR_ENABLED.store(before, Ordering::Relaxed);
    }

    /// Light text must not be emitted on a light ground for any semantic role.
    /// In the light theme the status bar is the one explicit ground and it is
    /// light, so every foreground used on it must be dark; body text uses the
    /// terminal's own light ground, so the primary foreground must be dark too.
    #[test]
    fn light_theme_emits_no_light_text_on_a_light_ground() {
        let _guard = lock();
        set_theme(Theme::Light);
        let palette = theme_accessors();

        // The status-bar ground is light; the text painted on it must not be.
        assert!(
            luminance(palette.status_bg) > 0.5,
            "light theme's status ground must be light"
        );
        for (name, fg) in [
            ("accent", palette.accent),
            ("user_color", palette.user_color),
            ("secondary", palette.secondary),
            ("success", palette.success),
            ("warning", palette.warning),
            ("danger", palette.danger),
            ("code_color", palette.code_color),
            ("foreground", palette.foreground),
        ] {
            assert!(
                luminance(fg) < 0.5,
                "{name} is light text on a light ground"
            );
        }

        // Text on the accent (selection badges, highlighted rows) must contrast
        // with the accent regardless of theme.
        assert!(
            contrast(palette.on_accent, palette.accent) >= 4.5,
            "on_accent must contrast with the accent in the light theme"
        );
    }

    /// The dark theme pairs a light ground (the terminal) with light text, so
    /// the property that still has to hold there is that accent-on-accent text
    /// contrasts — the one fixed ground the TUI paints itself.
    #[test]
    fn dark_theme_accent_text_contrasts_with_the_accent() {
        let _guard = lock();
        set_theme(Theme::Dark);
        let palette = theme_accessors();
        assert!(
            contrast(palette.on_accent, palette.accent) >= 4.5,
            "on_accent must contrast with the accent in the dark theme"
        );
    }

    #[test]
    fn resolve_theme_auto_honours_colorfgbg_background_and_falls_back_to_dark() {
        // A light background (>= 7) selects the light palette.
        assert_eq!(resolve_theme(ThemeChoice::Auto, Some("15;7")), Theme::Light);
        assert_eq!(resolve_theme(ThemeChoice::Auto, Some("0;15")), Theme::Light);
        // A dark background (0–6) keeps the dark palette.
        assert_eq!(resolve_theme(ThemeChoice::Auto, Some("15;0")), Theme::Dark);
        assert_eq!(resolve_theme(ThemeChoice::Auto, Some("12;6")), Theme::Dark);
        // Absent or unparseable falls back to dark rather than guessing.
        assert_eq!(resolve_theme(ThemeChoice::Auto, None), Theme::Dark);
        assert_eq!(
            resolve_theme(ThemeChoice::Auto, Some("default;default")),
            Theme::Dark
        );
        assert_eq!(
            resolve_theme(ThemeChoice::Auto, Some("garbage")),
            Theme::Dark
        );
        // Explicit choices ignore the environment.
        assert_eq!(resolve_theme(ThemeChoice::Light, None), Theme::Light);
        assert_eq!(resolve_theme(ThemeChoice::Dark, Some("15;7")), Theme::Dark);
    }

    #[test]
    fn color_decision_follows_choice_env_and_tty() {
        let _guard = lock();
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

    /// Relative luminance per WCAG sRGB. `Reset` is the terminal default and
    /// is treated as mid-grey so an unset colour is never misread as "light" or
    /// "dark" by these checks.
    fn luminance(color: Color) -> f64 {
        let (r, g, b) = match color {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::White => (255, 255, 255),
            Color::Black => (0, 0, 0),
            _ => return 0.5,
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    fn channel(v: u8) -> f64 {
        let s = v as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powi(2)
        }
    }

    /// WCAG contrast ratio between two colours.
    fn contrast(a: Color, b: Color) -> f64 {
        let (hi, lo) = {
            let la = luminance(a);
            let lb = luminance(b);
            if la >= lb { (la, lb) } else { (lb, la) }
        };
        (hi + 0.05) / (lo + 0.05)
    }
}
