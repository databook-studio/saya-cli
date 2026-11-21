//! The full-screen front: the transcript view model a turn's stream folds
//! into, and the pane that paints it.
//!
//! The model is presentation-free — no terminal library, no widget, no glyph —
//! so a turn can be folded and asserted on without a terminal.

mod view;

// This `pub(crate)` surface is the view model's boundary: the pane above it
// consumes it. Nothing in the crate references it yet, so the re-exports read
// as unused in a lib build — they are the boundary, not dead code.
#[allow(unused_imports)]
pub(crate) use view::{
    Activity, Answer, Entry, Notice, ResultPreview, Step, StepState, TranscriptView, TurnView,
    UserMessage, apply,
};
