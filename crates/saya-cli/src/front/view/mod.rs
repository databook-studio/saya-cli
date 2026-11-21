//! The transcript view model: what a turn's stream folds into, and the pure
//! reducer that folds it.

mod entry;
mod entry_blocks;
mod entry_result;
mod reduce;
mod steps;
mod steps_sql;

// The view model's boundary: the pane above it reads these, so nothing in this
// module is referenced by its own build yet.
#[allow(unused_imports)]
pub(crate) use entry::{
    Activity, Answer, Entry, Notice, Step, StepState, TranscriptView, TurnView, UserMessage,
};
#[allow(unused_imports)]
pub(crate) use entry_blocks::{
    Clarification, Command, CommandState, DiffKind, DiffLine, Edit, Error, Helper, HelperState,
    Stop,
};
#[allow(unused_imports)]
pub(crate) use entry_result::ResultPreview;
#[allow(unused_imports)]
pub(crate) use reduce::apply;
#[allow(unused_imports)]
pub(crate) use steps::{elapsed_text, folded_summary, own_words as command_words, step_line};
#[allow(unused_imports)]
pub(crate) use steps_sql::one_line as sql_one_line;

#[cfg(test)]
#[path = "reduce_fixture.rs"]
mod reduce_fixture;

#[cfg(test)]
#[path = "reduce_blocks_tests.rs"]
mod reduce_blocks_tests;

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
