//! The new front end: the TUI that replaces `interactive::tui` one screen at a
//! time — its theme (semantic tokens, glyphs, the `--ascii` fallback) and the
//! transcript view model a turn's stream folds into.
//!
//! Both are presentation-free data so they can be asserted on without a
//! terminal; the panes that paint them come later.

pub(crate) mod tui;
mod view;

// This `pub(crate)` surface is the view model's boundary: the pane above it
// consumes it. Nothing in the crate references it yet, so the re-exports read
// as unused in a lib build — they are the boundary, not dead code.
#[allow(unused_imports)]
pub(crate) use view::{
    Activity, Answer, Entry, Notice, ResultPreview, Step, StepState, TranscriptView, TurnView,
    UserMessage, apply,
};
