//! The reading surface's kept renderers: an answer's markdown, and the starter
//! questions the welcome pane offers.
//!
//! The old panes — the transcript, the splash, and the empty state's frame —
//! went with the old surface in X1; the new surface paints its own from
//! `front/tui/view`, reading these two leaves rather than copying them.

mod markdown;
mod starter;

// The new front's transcript renders the answer body with this renderer, not a
// second copy of it (`06` §5, T5).
pub(crate) use markdown::markdown_spans_fenced;
pub(crate) use starter::starter_questions;

