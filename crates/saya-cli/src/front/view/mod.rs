//! The transcript view model: what a turn's stream folds into, and the pure
//! reducer that folds it.

mod entry;
mod reduce;

// The view model's boundary: the pane above it reads these, so nothing in this
// module is referenced by its own build yet.
#[allow(unused_imports)]
pub(crate) use entry::{
    Activity, Answer, Entry, Notice, ResultPreview, Step, StepState, TranscriptView, TurnView,
    UserMessage,
};
#[allow(unused_imports)]
pub(crate) use reduce::apply;

#[cfg(test)]
#[path = "reduce_fixture.rs"]
mod reduce_fixture;

#[cfg(test)]
#[path = "reduce_tests.rs"]
mod reduce_tests;
