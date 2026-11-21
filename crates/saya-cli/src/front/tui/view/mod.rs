//! The new TUI's panes (`06` §5, units T8-T13).
//!
//! T9 lands the docked card: the call the operator has to answer, laid out as
//! the rows a pane paints — the caller's title, the call's facts verbatim, why
//! it is asked, the keys that answer it, and, for a clarification, the field
//! the answer is typed into.

pub(crate) mod cards;

// This `pub(crate)` surface is the pane layer's boundary: the runtime and the
// panes above read it. Nothing in the crate references it yet, so the
// re-exports read as unused in a lib build — they are the boundary, not dead
// code.
#[allow(unused_imports)]
pub(crate) use cards::{BODY_ROWS, Card, Kind, approval, clarification};

#[cfg(test)]
#[path = "cards_fixture.rs"]
pub(crate) mod cards_fixture;

#[cfg(test)]
#[path = "cards_tests.rs"]
mod cards_tests;
