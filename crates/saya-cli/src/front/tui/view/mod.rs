//! The screens the new TUI paints, one unit at a time (`06` §5, T4-T18).
//!
//! Every screen is drawn from the model and returns plain rows, so a test can
//! assert a whole box without a terminal. The composer (`02` §7) is the first:
//! its state table and copy in `composer`, the box it is drawn in in
//! `composer_frame`.

mod composer;
mod composer_frame;

// This `pub(crate)` surface is the composer's boundary: the runtime above it
// draws these rows, and nothing in the crate reads them yet — they are the
// boundary, not dead code.
#[allow(unused_imports)]
pub(crate) use composer::{
    COMPACT_IDLE_HINT, COMPACT_WIDTH, IDLE_HINT, QUEUED_HINT, Row, Run, State, hint, placeholder,
    state, title,
};
#[allow(unused_imports)]
pub(crate) use composer_frame::rows;

#[cfg(test)]
#[path = "composer_tests.rs"]
mod composer_tests;
