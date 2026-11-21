//! The new TUI's panes (`06` §5, units T4, T8-T13): one function per region of
//! the frame, each a pure mapping from the facts the surface holds to what it
//! hands back.
//!
//! T4 lands the status line: one row that names the mode in force, what the
//! session can touch and what the surface is doing. What it may show and what
//! gives way when the terminal is narrow are two rules, so they are two files.
//!
//! T9 lands the docked card: the call the operator has to answer, laid out as
//! the rows a pane paints — the caller's title, the call's facts verbatim, why
//! it is asked, the keys that answer it, and, for a clarification, the field
//! the answer is typed into.

pub(crate) mod cards;
mod status;
mod status_fit;
mod status_segments;

// This `pub(crate)` surface is the pane layer's boundary: the runtime and the
// panes above read it. Nothing in the crate references it yet, so the
// re-exports read as unused in a lib build — they are the boundary, not dead
// code.
#[allow(unused_imports)]
pub(crate) use cards::{BODY_ROWS, Card, Kind, approval, clarification};
#[allow(unused_imports)]
pub(crate) use status::{ActivityState, StatusFacts, status_line};

#[cfg(test)]
#[path = "cards_fixture.rs"]
pub(crate) mod cards_fixture;

#[cfg(test)]
#[path = "cards_tests.rs"]
mod cards_tests;

#[cfg(test)]
#[path = "status_fixture.rs"]
mod status_fixture;

#[cfg(test)]
#[path = "status_fit_tests.rs"]
mod status_fit_tests;

#[cfg(test)]
#[path = "status_tests.rs"]
mod status_tests;
