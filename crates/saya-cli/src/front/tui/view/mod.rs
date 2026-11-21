//! The new TUI's panes: one function per region of the frame, each a pure
//! mapping from the facts the surface holds to the spans it hands back.
//!
//! The status line is the first: one row that names the mode in force, what the
//! session can touch and what the surface is doing. What it may show and what
//! gives way when the terminal is narrow are two rules, so they are two files.

mod status;
mod status_fit;
mod status_segments;

// This `pub(crate)` surface is the pane's boundary: the runtime renders what it
// returns. Nothing in the crate calls it yet, so the re-exports read as unused
// in a lib build — they are the boundary, not dead code.
#[allow(unused_imports)]
pub(crate) use status::{ActivityState, StatusFacts, status_line};

#[cfg(test)]
#[path = "status_fixture.rs"]
mod status_fixture;

#[cfg(test)]
#[path = "status_fit_tests.rs"]
mod status_fit_tests;

#[cfg(test)]
#[path = "status_tests.rs"]
mod status_tests;
