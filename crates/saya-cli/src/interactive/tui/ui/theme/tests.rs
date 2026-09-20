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
