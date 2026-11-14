use super::*;

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
