use super::*;

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
        ((s + 0.055) / 1.055).powf(2.4)
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

/// The helpers are WCAG 2.x relative luminance and contrast, pinned to a
/// published reference: `#767676` on white is 4.54:1, the canonical darkest
/// grey that still passes AA for body text. An approximation of the sRGB
/// transfer curve (a square instead of the 2.4 exponent) understates every
/// mid-tone's contrast and would push the palette darker than it needs to be.
#[test]
fn the_contrast_helper_is_wcag() {
    let ratio = contrast(Color::Rgb(0x76, 0x76, 0x76), Color::Rgb(255, 255, 255));
    assert!(
        (ratio - 4.54).abs() < 0.01,
        "#767676 on white must be 4.54:1 (WCAG), got {ratio:.3}"
    );
}
