//! The reading surface: transcript, markdown, splash, and empty state.

mod empty_state;
mod markdown;
mod splash;
mod starter;
mod transcript;

#[cfg(test)]
#[path = "transcript_draft_tests.rs"]
mod transcript_draft_tests;

pub(super) use empty_state::draw_empty_state;
// The new front's transcript renders the answer body with this renderer, not a
// second copy of it (`06` §5, T5).
pub(crate) use markdown::markdown_spans_fenced;
#[cfg(test)]
pub(in crate::interactive::tui) use splash::DATABASE_NONE;
pub(crate) use starter::starter_questions;
pub(super) use transcript::draw_transcript;
pub(in crate::interactive::tui) use transcript::{SPINNER, unlabelled_glyph};
