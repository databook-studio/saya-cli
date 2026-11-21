//! The raw hues the two palettes are made of, defined once.
//!
//! [`palette`](crate::interactive::tui::ui::theme::palette) paints the current
//! TUI from these; the new TUI's semantic tokens
//! ([`tokens`](crate::front::tui::theme::tokens)) read the same constants, so a
//! hue has one definition and a hard-coded colour anywhere else stays the bug
//! centralisation exists to prevent. Nothing here reads or writes the process
//! globals the palette resolves at session start: these are data, and a caller
//! that needs the colour switch asks `palette`.

use ratatui::style::Color;

/// saya's signature accent (iris violet): brand, assistant, focus, borders.
pub(crate) const ACCENT_DARK: Color = Color::Rgb(157, 139, 245);
/// The deeper light-terminal iris, so the accent still reads on white.
pub(crate) const ACCENT_LIGHT: Color = Color::Rgb(109, 78, 200);
/// User turns: a cool secondary so the accent stays saya's.
pub(crate) const INFO_DARK: Color = Color::Rgb(106, 155, 204);
/// The light-terminal counterpart of [`INFO_DARK`].
pub(crate) const INFO_LIGHT: Color = Color::Rgb(44, 108, 176);
/// Secondary/de-emphasised text: captions, footers, NULLs, tool lines.
pub(crate) const DIM_DARK: Color = Color::Rgb(168, 162, 154);
/// The light-terminal counterpart of [`DIM_DARK`].
pub(crate) const DIM_LIGHT: Color = Color::Rgb(107, 102, 96);
/// Status: success / safe.
pub(crate) const OK_DARK: Color = Color::Rgb(127, 174, 107);
/// The light-terminal counterpart of [`OK_DARK`].
pub(crate) const OK_LIGHT: Color = Color::Rgb(61, 122, 53);
/// Status: error / danger, and the bypass chip's ground.
pub(crate) const BAD_DARK: Color = Color::Rgb(229, 105, 95);
/// The light-terminal counterpart of [`BAD_DARK`].
pub(crate) const BAD_LIGHT: Color = Color::Rgb(181, 55, 44);
/// Status: caution (caveats, paused).
pub(crate) const WARN_DARK: Color = Color::Rgb(224, 164, 88);
/// The light-terminal counterpart of [`WARN_DARK`].
pub(crate) const WARN_LIGHT: Color = Color::Rgb(154, 93, 18);
/// Inline `code` and SQL.
pub(crate) const CODE_DARK: Color = Color::Rgb(127, 181, 214);
/// The light-terminal counterpart of [`CODE_DARK`].
pub(crate) const CODE_LIGHT: Color = Color::Rgb(46, 109, 158);
/// The status line's and a card's ground (faint iris-tinted dark).
pub(crate) const SURFACE_DARK: Color = Color::Rgb(30, 28, 36);
/// The light-terminal counterpart of [`SURFACE_DARK`].
pub(crate) const SURFACE_LIGHT: Color = Color::Rgb(236, 233, 245);
/// Primary foreground on a known-light terminal. The dark palette's foreground
/// is the terminal's own text colour (`Color::Reset`), deliberately not a fixed
/// white: a wrong dark/light guess can then never paint the answer invisible.
pub(crate) const FOREGROUND_LIGHT: Color = Color::Rgb(40, 38, 42);
