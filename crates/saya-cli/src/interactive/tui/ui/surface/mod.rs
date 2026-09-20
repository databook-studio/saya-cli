//! The reading surface: transcript, markdown, splash, and empty state.

mod empty_state;
mod markdown;
mod splash;
mod transcript;

pub(super) use empty_state::draw_empty_state;
#[cfg(test)]
pub(crate) use splash::{NO_DATABASE_HEADLINE, NO_WORKSPACE_LINES};
pub(super) use transcript::draw_transcript;
pub(in crate::interactive::tui) use transcript::{SPINNER, unlabelled_glyph};
