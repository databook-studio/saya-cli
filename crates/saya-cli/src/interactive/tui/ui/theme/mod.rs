//! The theme leaves the new surface reuses: the hue constants, the palette
//! choice, and the terminal's own answer for `auto`.
//!
//! The old surface's painting — the transcript block styles and the accessors
//! only its chrome read — went with it in X1. What is left is what the kept
//! markdown renderer and the launch read: which ground is in force, whether
//! colour is on, and the hues themselves.

mod detect;
pub(crate) mod hue;
mod palette;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub(crate) use detect::{probe_terminal_theme, resolve_startup_theme};
pub(crate) use palette::{Theme, decide_public, set_color_enabled, set_theme};
pub(in crate::interactive::tui) use palette::{accent, code_color, foreground};
