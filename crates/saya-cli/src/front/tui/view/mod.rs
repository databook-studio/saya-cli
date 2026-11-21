//! The new TUI's panes (`06` §5, units T4-T18): one function per region of the
//! frame, each a pure mapping from the facts the surface holds to the rows it
//! hands back, so a test can assert a whole region without a terminal.
//!
//! T4 lands the status line: one row that names the mode in force, what the
//! session can touch and what the surface is doing. What it may show and what
//! gives way when the terminal is narrow are two rules, so they are two files.
//!
//! T8 lands the composer (`02` §7): its state table and copy in `composer`, the
//! box it is drawn in in `composer_frame`.
//!
//! T9 lands the docked card: the call the operator has to answer, laid out as
//! the rows a pane paints — the caller's title, the call's facts verbatim, why
//! it is asked, the keys that answer it, and, for a clarification, the field
//! the answer is typed into.
//!
//! T5 lands the transcript: the conversation as the rows a terminal paints,
//! with the newest turn at the live edge. What one turn reads as is `turn`;
//! how the turns sit under each other is `transcript`.
//!
//! T10 lands the command palette (`02` §11): the catalogue rows a query fuzzy-matches
//! through their names and their aliases, the selected row's usage, and the box they
//! are painted in. The list is `slash/catalog`'s, read rather than copied.
//!
//! T7 lands the transcript's other blocks (`02` §3.4-§3.10): the turn's work —
//! an edit's diff in `blocks`, a command, a helper — and what ends it, in
//! `blocks_stop` (an error, a budget, a pause, a cancel) and `blocks_question`
//! (the model's question and the cue that answers it in the composer).

mod blocks;
mod blocks_question;
mod blocks_stop;
pub(crate) mod cards;
mod composer;
mod composer_frame;
mod palette;
mod palette_frame;
mod palette_match;
mod palette_rows;
mod status;
mod status_fit;
mod status_segments;
mod transcript;
mod turn;
mod turn_activity;
mod turn_preview;
mod turn_rows;

// This `pub(crate)` surface is the pane layer's boundary: the runtime and the
// panes above read it. Nothing in the crate references it yet, so the
// re-exports read as unused in a lib build — they are the boundary, not dead
// code.
#[allow(unused_imports)]
pub(crate) use cards::{BODY_ROWS, Card, Kind, approval, clarification};
#[allow(unused_imports)]
pub(crate) use composer::{
    COMPACT_IDLE_HINT, COMPACT_WIDTH, IDLE_HINT, QUEUED_HINT, Row, Run, State, hint, placeholder,
    state, title,
};
#[allow(unused_imports)]
pub(crate) use composer_frame::rows;
#[allow(unused_imports)]
pub(crate) use palette::Palette;
#[allow(unused_imports)]
pub(crate) use palette_rows::rows as palette_rows;
#[allow(unused_imports)]
pub(crate) use status::{ActivityState, StatusFacts, status_line};
#[allow(unused_imports)]
pub(crate) use transcript::{TranscriptFacts, rows as transcript_rows};

#[cfg(test)]
#[path = "cards_fixture.rs"]
pub(crate) mod cards_fixture;

#[cfg(test)]
#[path = "blocks_fixture.rs"]
mod blocks_fixture;

#[cfg(test)]
#[path = "blocks_stop_tests.rs"]
mod blocks_stop_tests;

#[cfg(test)]
#[path = "blocks_tests.rs"]
mod blocks_tests;

#[cfg(test)]
#[path = "cards_tests.rs"]
mod cards_tests;

#[cfg(test)]
#[path = "composer_tests.rs"]
mod composer_tests;

#[cfg(test)]
#[path = "palette_fixture.rs"]
mod palette_fixture;

#[cfg(test)]
#[path = "palette_match_tests.rs"]
mod palette_match_tests;

#[cfg(test)]
#[path = "palette_tests.rs"]
mod palette_tests;

#[cfg(test)]
#[path = "status_fixture.rs"]
mod status_fixture;

#[cfg(test)]
#[path = "status_fit_tests.rs"]
mod status_fit_tests;

#[cfg(test)]
#[path = "status_tests.rs"]
mod status_tests;

#[cfg(test)]
#[path = "transcript_fixture.rs"]
mod transcript_fixture;

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod transcript_tests;
