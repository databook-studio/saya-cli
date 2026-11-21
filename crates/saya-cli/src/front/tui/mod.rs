//! The new TUI (`06` §5, units T0-T20).
//!
//! T0 landed the theme: semantic tokens, the glyph set and the `--ascii`
//! fallback as pure data, so the screens that follow name meaning instead of
//! hue. T3 lands the model, the messages it takes, the pure update that folds
//! them and the keymap that reads a press: one owner of state, and every
//! decision that leaves the surface a [`Cmd`] the runtime performs. T4 lands the
//! status line, the first pane: one pure function from the session's facts and a
//! width to the row a terminal paints. No terminal I/O, no widgets beyond the
//! line the pane builds itself, no drawing loop — the panes come later.
//!
//! T14 lands the runtime: the queue the producers fill (the input thread, the
//! session's front, the tick), the loop that folds every message waiting in it
//! through the pure update, the executor that performs the commands, and the
//! frame painted only when the model changed and at most thirty times a second
//! — the panes T4-T13 built, composed into one screen.

pub(crate) mod cards_keys;
pub(crate) mod keys;
pub(crate) mod model;
pub(crate) mod model_overlay;
pub(crate) mod msg;
pub(crate) mod run;
pub(crate) mod run_clock;
pub(crate) mod run_cmd;
pub(crate) mod run_draw;
pub(crate) mod run_draw_card;
pub(crate) mod run_draw_overlay;
pub(crate) mod run_front;
pub(crate) mod run_input;
pub(crate) mod run_panic;
pub(crate) mod run_screen;
pub(crate) mod theme;
pub(crate) mod update;
pub(crate) mod view;

// This `pub(crate)` surface is the surface's boundary: the runtime above it
// reads the model, feeds it messages and performs the commands it answers
// with. Nothing in the crate references it yet, so the re-exports read as
// unused in a lib build — they are the boundary, not dead code.
#[allow(unused_imports)]
pub(crate) use cards_keys::{CARD_PAGE_ROWS, CardIntent, card_intent};
#[allow(unused_imports)]
pub(crate) use keys::{CARD_HOLDS_MODE, Intent, intent};
#[allow(unused_imports)]
pub(crate) use model::Model;
#[allow(unused_imports)]
pub(crate) use model_overlay::Overlay;
#[allow(unused_imports)]
pub(crate) use msg::{Cmd, Msg, Worker};
#[allow(unused_imports)]
pub(crate) use run::Runtime;
#[allow(unused_imports)]
pub(crate) use run::run;
#[allow(unused_imports)]
pub(crate) use run_clock::{Clock, SystemClock};
#[allow(unused_imports)]
pub(crate) use run_cmd::{CmdPort, SessionPort, perform};
#[allow(unused_imports)]
pub(crate) use run_draw::{Facts, Regions};
#[allow(unused_imports)]
pub(crate) use run_front::Forward;
#[allow(unused_imports)]
pub(crate) use run_screen::{Screen, TtyScreen};
#[allow(unused_imports)]
pub(crate) use theme::{Charset, Glyph, Scheme, Token, bypass_chip, charset, form, glyph, style};
#[allow(unused_imports)]
pub(crate) use update::update;

#[cfg(test)]
#[path = "cards_keys_tests.rs"]
mod cards_keys_tests;

#[cfg(test)]
#[path = "update_fixture.rs"]
mod update_fixture;

#[cfg(test)]
#[path = "update_tests.rs"]
mod update_tests;

#[cfg(test)]
#[path = "update_mode_tests.rs"]
mod update_mode_tests;

#[cfg(test)]
#[path = "update_cycle_tests.rs"]
mod update_cycle_tests;

#[cfg(test)]
#[path = "run_fixture.rs"]
pub(crate) mod run_fixture;

#[cfg(test)]
#[path = "run_fixture_port.rs"]
pub(crate) mod run_fixture_port;

#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;

#[cfg(test)]
#[path = "run_mode_tests.rs"]
mod run_mode_tests;

#[cfg(test)]
#[path = "run_draw_tests.rs"]
mod run_draw_tests;

#[cfg(test)]
#[path = "run_input_tests.rs"]
mod run_input_tests;

#[cfg(test)]
#[path = "run_front_tests.rs"]
mod run_front_tests;
