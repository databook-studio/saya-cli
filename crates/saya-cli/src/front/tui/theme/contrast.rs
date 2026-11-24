//! WCAG 2.1 contrast, measured instead of guessed.
//!
//! `02` §12 rule 3 asks for a contrast floor but marks its values
//! **UNVERIFIED** on real terminals, so the rule is computed here from the
//! palette's own hues and asserted by the theme's tests:
//!
//! * **ground** — the terminal's default background: black for the dark
//!   palette, white for the light one. The dark `fg`/`user` forms resolve to
//!   [`Color::Reset`], i.e. the terminal's own text on its own ground, and the
//!   light palette is only chosen when the terminal is known light.
//! * **floor** — 4.5:1 for every hue-bearing form (WCAG AA body text) and 3:1
//!   for `dim` (AA large text: captions and NULLs are secondary, not body copy).
//! * **on the surface** — a form that paints on `surface` (the status line, a
//!   card) keeps 3:1 against it, so a row is never unreadable on its own ground.
//! * a `Reset` form paints no hue of its own and is exempt: there is nothing to
//!   measure.

use ratatui::style::Color;

/// The dark palette's presumed terminal ground.
pub(crate) const GROUND_DARK: Color = Color::Rgb(0, 0, 0);
/// The light palette's presumed terminal ground.
pub(crate) const GROUND_LIGHT: Color = Color::Rgb(255, 255, 255);
/// Floor for body text: WCAG 2.1 AA normal text.
pub(crate) const FLOOR_TEXT: f64 = 4.5;
/// Floor for `dim` text: WCAG 2.1 AA large text.
pub(crate) const FLOOR_DIM: f64 = 3.0;
/// Floor for a hue that paints on `surface` rather than on the ground.
pub(crate) const FLOOR_ON_SURFACE: f64 = 3.0;

/// WCAG 2.1 relative luminance, on the 0.0–1.0 scale with 0.0 black.
///
/// `None` for a colour that carries no measurable channels — `Reset` and the
/// terminal-index colours, whose value only the terminal knows. A caller that
/// gets `None` has no luminance to check and must say so rather than assume.
pub(crate) fn relative_luminance(colour: Color) -> Option<f64> {
    let Color::Rgb(r, g, b) = colour else {
        return None;
    };
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b))
}

/// WCAG 2.1 contrast ratio between two colours, from 1.0 (identical) to 21.0
/// (black on white). `None` when either colour has no measurable luminance.
pub(crate) fn contrast_ratio(first: Color, second: Color) -> Option<f64> {
    let (first, second) = (relative_luminance(first)?, relative_luminance(second)?);
    let (lighter, darker) = if first > second {
        (first, second)
    } else {
        (second, first)
    };
    Some((lighter + 0.05) / (darker + 0.05))
}
