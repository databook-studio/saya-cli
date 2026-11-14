use super::*;

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
