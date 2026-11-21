//! The new TUI (`06` §5, units T0-T20).
//!
//! T0 landed the theme: semantic tokens, the glyph set and the `--ascii`
//! fallback as pure data, so the screens that follow name meaning instead of
//! hue. T3 lands the model, the messages it takes, the pure update that folds
//! them and the keymap that reads a press: one owner of state, and every
//! decision that leaves the surface a [`Cmd`] the runtime performs. No
//! terminal I/O, no widgets, no drawing — the panes come later.

pub(crate) mod keys;
pub(crate) mod model;
pub(crate) mod msg;
pub(crate) mod theme;
pub(crate) mod update;
pub(crate) mod view;

// This `pub(crate)` surface is the surface's boundary: the runtime above it
// reads the model, feeds it messages and performs the commands it answers
// with. Nothing in the crate references it yet, so the re-exports read as
// unused in a lib build — they are the boundary, not dead code.
#[allow(unused_imports)]
pub(crate) use keys::{CARD_HOLDS_MODE, Intent, intent};
#[allow(unused_imports)]
pub(crate) use model::Model;
#[allow(unused_imports)]
pub(crate) use msg::{Cmd, Msg, Worker};
#[allow(unused_imports)]
pub(crate) use theme::{Charset, Glyph, Scheme, Token, bypass_chip, charset, form, glyph, style};
#[allow(unused_imports)]
pub(crate) use update::update;

#[cfg(test)]
#[path = "update_fixture.rs"]
mod update_fixture;

#[cfg(test)]
#[path = "update_tests.rs"]
mod update_tests;
