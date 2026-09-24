use super::{
    accent, code_color, danger, foreground, on_accent, secondary, status_bg, success, user_color,
    warning,
};
use ratatui::style::Color;

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

use super::{COLOR_ENABLED, Theme, decide, resolve_theme, set_theme};
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
/// contrast tests below still have to pass for the new values. `foreground`
/// is `Reset` — the terminal's own text colour — never a fixed white, so a
/// wrong dark/light guess can never paint the answer invisible.
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
    assert_eq!(palette.foreground, Color::Reset);
    assert_eq!(palette.on_accent, Color::Black);
}

/// Snapshot the light palette. The foregrounds are all deeper than their
/// dark counterparts so they read on a light ground; several entries were
/// darkened from their original hue to clear the 4.5:1 contrast tests below.
#[test]
fn light_palette_is_pinned() {
    let _guard = lock();
    set_theme(Theme::Light);
    let palette = theme_accessors();
    assert_eq!(palette.accent, Color::Rgb(100, 67, 197));
    assert_eq!(palette.user_color, Color::Rgb(41, 99, 162));
    assert_eq!(palette.secondary, Color::Rgb(93, 89, 83));
    assert_eq!(palette.success, Color::Rgb(54, 108, 47));
    assert_eq!(palette.warning, Color::Rgb(139, 84, 16));
    assert_eq!(palette.danger, Color::Rgb(181, 55, 44));
    assert_eq!(palette.status_bg, Color::Rgb(236, 233, 245));
    assert_eq!(palette.code_color, Color::Rgb(43, 101, 147));
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

/// WCAG AA body-text threshold used throughout these tests.
const AA: f64 = 4.5;

/// Solarized Light cream — the second light ground the light palette must
/// read on, alongside pure white.
const CREAM: Color = Color::Rgb(253, 246, 227);

/// A dark terminal background one shade off pure black — the second dark
/// ground the dark palette must read on, alongside pure black.
const DARK_GROUND: Color = Color::Rgb(30, 30, 30);

/// Every light-palette text accessor must clear AA contrast against a white
/// terminal ground *and* against Solarized Light's cream: cream is the
/// tighter of the two (it is slightly darker than pure white), so it is the
/// one the luminance guard this test replaces could not see.
#[test]
fn light_palette_text_meets_aa_contrast_on_white_and_cream() {
    let _guard = lock();
    set_theme(Theme::Light);
    let palette = theme_accessors();
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
            contrast(fg, Color::White) >= AA,
            "{name} fails AA contrast on white"
        );
        assert!(
            contrast(fg, CREAM) >= AA,
            "{name} fails AA contrast on cream"
        );
    }
}

/// Every dark-palette text accessor with a concrete RGB must clear AA
/// contrast against black and against a near-black terminal ground.
/// `foreground` is `Color::Reset` in the dark palette by design — it has no
/// RGB to measure, since it now paints the terminal's own text colour
/// instead of a fixed one — so it is skipped here and pinned separately by
/// [`dark_foreground_is_the_terminals_own_colour`].
#[test]
fn dark_palette_text_meets_aa_contrast_on_black_and_dark_ground() {
    let _guard = lock();
    set_theme(Theme::Dark);
    let palette = theme_accessors();
    for (name, fg) in [
        ("accent", palette.accent),
        ("user_color", palette.user_color),
        ("secondary", palette.secondary),
        ("success", palette.success),
        ("warning", palette.warning),
        ("danger", palette.danger),
        ("code_color", palette.code_color),
    ] {
        assert!(
            contrast(fg, Color::Black) >= AA,
            "{name} fails AA contrast on black"
        );
        assert!(
            contrast(fg, DARK_GROUND) >= AA,
            "{name} fails AA contrast on a near-black ground"
        );
    }
}

/// The one set of accessors actually painted on `status_bg()` today: the
/// status bar's database-name segment (`accent`, bold) and its model/mode/
/// tasks segments plus the whole bar's fallback style (`secondary`) — see
/// `ui/chrome/status.rs` (`bar`, `bg`) and `ui/chrome/status_segments.rs`
/// (`bar_spans`' `base.fg(...)`). `success`/`warning` were painted there
/// before the status bar split into its own tint and no longer are (see
/// `ui/chrome/context_line.rs`, which paints them on the terminal's own
/// ground, not `status_bg`), so they are intentionally absent here.
#[test]
fn status_bg_painted_text_meets_aa_contrast_in_both_themes() {
    let _guard = lock();
    for theme in [Theme::Dark, Theme::Light] {
        set_theme(theme);
        let palette = theme_accessors();
        assert!(
            contrast(palette.accent, palette.status_bg) >= AA,
            "{theme:?}: accent fails AA contrast on status_bg"
        );
        assert!(
            contrast(palette.secondary, palette.status_bg) >= AA,
            "{theme:?}: secondary fails AA contrast on status_bg"
        );
    }
}

/// Text on the accent (selection badges, highlighted rows) must contrast
/// with the accent in both themes — the one ground the TUI paints itself
/// regardless of the terminal's own colours.
#[test]
fn on_accent_contrasts_with_the_accent_in_both_themes() {
    let _guard = lock();
    for theme in [Theme::Dark, Theme::Light] {
        set_theme(theme);
        let palette = theme_accessors();
        assert!(
            contrast(palette.on_accent, palette.accent) >= AA,
            "{theme:?}: on_accent fails AA contrast on accent"
        );
    }
}

/// The dark palette's body text is the terminal's own colour, never a fixed
/// white: this is the safety net that keeps a wrong dark/light guess from
/// ever painting the answer invisible on a light terminal.
#[test]
fn dark_foreground_is_the_terminals_own_colour() {
    let _guard = lock();
    set_theme(Theme::Dark);
    assert_eq!(foreground(), Color::Reset);
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
