//! The theme choice, pinned: the `auto` heuristic's one rule, and the terminal
//! answer that overrides it ([`super::detect`]'s own tests).

use super::palette::{Theme, resolve_theme};
use saya_config::ThemeChoice;

/// `auto` honours `COLORFGBG` when the terminal publishes it — a light
/// background (index ≥ 7) selects the light ground — and falls back to dark
/// when the value is absent or unparseable, never guessing.
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
