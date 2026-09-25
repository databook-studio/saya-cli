use super::{PROBE_TIMEOUT, Theme, detect_theme, resolve_startup_theme};
use saya_config::ThemeChoice;
use std::time::Duration;

#[test]
fn auto_takes_the_terminal_answer() {
    assert_eq!(
        detect_theme(ThemeChoice::Auto, || Some(Theme::Light), None),
        Theme::Light
    );
    // The terminal's own answer wins even when COLORFGBG disagrees (a
    // light background by env, but the probe says Dark).
    assert_eq!(
        detect_theme(ThemeChoice::Auto, || Some(Theme::Dark), Some("0;15")),
        Theme::Dark
    );
}

#[test]
fn auto_without_an_answer_falls_back_to_colorfgbg() {
    assert_eq!(
        detect_theme(ThemeChoice::Auto, || None, Some("0;15")),
        Theme::Light
    );
    assert_eq!(detect_theme(ThemeChoice::Auto, || None, None), Theme::Dark);
}

#[test]
fn an_explicit_theme_never_queries() {
    let never = || -> Option<Theme> { panic!("Dark/Light must never query the terminal") };
    assert_eq!(detect_theme(ThemeChoice::Dark, never, None), Theme::Dark);

    let never = || -> Option<Theme> { panic!("Dark/Light must never query the terminal") };
    assert_eq!(detect_theme(ThemeChoice::Light, never, None), Theme::Light);
}

#[test]
fn a_disabled_colour_or_non_tty_never_queries() {
    let never = || -> Option<Theme> { panic!("a gated Auto must never query the terminal") };
    // Colour disabled, otherwise a real terminal.
    assert_eq!(
        resolve_startup_theme(ThemeChoice::Auto, false, true, never, Some("0;15")),
        Theme::Light
    );

    let never = || -> Option<Theme> { panic!("a gated Auto must never query the terminal") };
    // Colour enabled, but stdout is not a terminal.
    assert_eq!(
        resolve_startup_theme(ThemeChoice::Auto, true, false, never, None),
        Theme::Dark
    );
}

#[test]
fn an_enabled_tty_auto_queries_the_terminal() {
    // The mirror of the gate tests above: colour on and a real terminal
    // both true, so the probe is reached and its answer wins.
    assert_eq!(
        resolve_startup_theme(ThemeChoice::Auto, true, true, || Some(Theme::Light), None),
        Theme::Light
    );
}

#[test]
fn probe_timeout_is_150_milliseconds() {
    assert_eq!(PROBE_TIMEOUT, Duration::from_millis(150));
}
