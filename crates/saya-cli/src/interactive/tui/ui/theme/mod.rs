//! Colour accessors for the TUI: accent, secondary, danger, and friends.
//!
//! The palette choice and colour switch live in [`palette`]; the tests live
//! in `tests.rs` via the `transcript/view.rs` `#[path]` pattern.

mod palette;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) use palette::Theme;
#[cfg(test)]
pub(in crate::interactive::tui) use palette::user_color;
pub(in crate::interactive::tui) use palette::{
    accent, centered, code_color, danger, foreground, kind_style, label_style, on_accent,
    secondary, status_bg, success, warning,
};
pub(crate) use palette::{decide_public, resolve_theme, set_color_enabled, set_theme};
// Test-only shims so the moved `tests.rs` keeps its `super::…` paths
// byte-identical: the theme's own tests are the only readers.
#[cfg(test)]
use palette::{COLOR_ENABLED, decide};
