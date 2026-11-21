//! The transcript view model: what a turn's stream folds into, and the pure
//! reducer that folds it.

mod entry;
mod reduce;
mod steps;
mod steps_sql;

// The view model's boundary: the pane above it reads these, so nothing in this
// module is referenced by its own build yet.
#[allow(unused_imports)]
pub(crate) use entry::{
    Activity, Answer, Entry, Notice, ResultPreview, Step, StepState, TranscriptView, TurnView,
    UserMessage,
};
#[allow(unused_imports)]
pub(crate) use reduce::apply;
#[allow(unused_imports)]
pub(crate) use steps::{folded_summary, step_line};
#[allow(unused_imports)]
pub(crate) use steps_sql::one_line as sql_one_line;

#[cfg(test)]
#[path = "reduce_fixture.rs"]
mod reduce_fixture;

#[cfg(test)]
#[path = "reduce_tests.rs"]
mod reduce_tests;

#[cfg(test)]
#[path = "steps_summary_tests.rs"]
mod steps_summary_tests;

#[cfg(test)]
#[path = "steps_sql_tests.rs"]
mod steps_sql_tests;

#[cfg(test)]
#[path = "steps_tests.rs"]
mod steps_tests;
