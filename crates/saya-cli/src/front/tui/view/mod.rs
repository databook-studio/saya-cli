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
//!
//! T11 lands the two things the surface says about itself: the contextual help sheet
//! (`02` §11) — the keys valid in the state the operator is in, read from one table
//! `help_keys` holds — and the toast (`02` §4), whose four seconds are counted in the
//! runtime's ticks.
//!
//! T12 lands the sessions picker (`02` §10): the saved sessions an operator
//! chooses from, each row's state read from the resume decision the chat layer
//! already makes, a filter over the title and the database, and a delete that
//! asks before it acts.
//!
//! T13 lands the empty transcript (`02` §9): one screen per way a session can have
//! nothing to show yet, with the examples a connected database suggests read from
//! today's `starter_questions`.
//!
//! T18 lands the mode picker (`02` §8): the six modes of the one consent axis on
//! one screen, each with the one line it means, and the words a change says —
//! §3.10's transcript line and §4's toast.
//!
//! T17 lands the background jobs drawer (`02` §8): one row per job with its
//! state's mark and its facts, where a job's call enters the same card slot as
//! the session's, and where `x` asks before it cancels.

mod blocks;
mod blocks_notice;
mod blocks_question;
mod blocks_stop;
pub(crate) mod cards;
mod cards_body;
mod cards_choices;
mod cards_diff;
mod cards_family;
mod cards_place;
pub(crate) mod changes;
mod composer;
mod composer_frame;
mod empty;
mod empty_lines;
pub(crate) mod empty_suggestions;
mod empty_welcome;
mod help;
mod help_keys;
mod help_rows_a;
mod help_rows_b;
mod highlight;
mod highlight_keywords;
mod highlight_langs;
mod inspector;
pub(crate) mod jobs;
pub(crate) mod jobs_rows;
mod mention;
mod meters;
pub(crate) mod mode_picker;
pub(crate) mod mode_words;
mod palette;
mod palette_frame;
mod palette_match;
mod palette_rows;
mod result;
mod result_table;
mod sessions;
mod sessions_cells;
mod sessions_row;
mod sessions_rows;
mod status;
mod status_fit;
mod status_segments;
mod toast;
mod transcript;
pub(crate) mod trust;
mod turn;
mod turn_activity;
mod turn_focus;
mod turn_rows;

// This `pub(crate)` surface is the pane layer's boundary: the runtime and the
// panes above read it. Nothing in the crate references it yet, so the
// re-exports read as unused in a lib build — they are the boundary, not dead
// code.
#[allow(unused_imports)]
pub(crate) use cards::{
    BODY_ROWS, Card, Choice, Family, Kind, approval, approval_waiting, cells, clarification,
};
#[allow(unused_imports)]
pub(crate) use cards_diff::{Ink, Row as CardRow};
#[allow(unused_imports)]
pub(crate) use changes::{ROWS as CHANGES_ROWS, TITLE as CHANGES_TITLE, rows as changes_rows};
#[allow(unused_imports)]
pub(crate) use composer::{
    COMPACT_IDLE_HINT, COMPACT_WIDTH, IDLE_HINT, QUEUED_HINT, Row, Run, State, hint, placeholder,
    state, title,
};
#[allow(unused_imports)]
pub(crate) use composer_frame::{fit, pad, rows};
#[allow(unused_imports)]
pub(crate) use empty::{Database, EmptyFacts, State as EmptyState, rows as empty_rows};
#[allow(unused_imports)]
pub(crate) use help::{
    Context, ROWS as HELP_ROWS, rows as help_rows, rows_scrolled as help_scrolled,
};
#[allow(unused_imports)]
pub(crate) use jobs::{CARD_FIRST, DETAIL_UNIT, Drawer, Job, NO_JOBS, State as JobState, TITLE};
#[allow(unused_imports)]
pub(crate) use jobs_rows::{
    Placement as JobsPlacement, SHEET_ROWS, SIDE_FROM, SIDE_WIDTH, placement as jobs_placement,
    rows as jobs_rows, side_width,
};
#[allow(unused_imports)]
pub(crate) use mention::{
    HINT as MENTION_HINT, MAX_VISIBLE as MENTION_ROWS, TITLE as MENTION_TITLE,
    height as mention_height, rows as mention_rows,
};
#[allow(unused_imports)]
pub(crate) use meters::{Meters, cost_text};
#[allow(unused_imports)]
pub(crate) use mode_picker::{ModePicker, ROWS as MODE_PICKER_ROWS, rows as mode_picker_rows};
#[allow(unused_imports)]
pub(crate) use palette::{Palette, ROWS as PALETTE_ROWS};
#[allow(unused_imports)]
pub(crate) use palette_rows::rows as palette_rows;
#[allow(unused_imports)]
pub(crate) use result::refusal as refusal_words;
#[allow(unused_imports)]
pub(crate) use result::rows as result_rows;
#[allow(unused_imports)]
pub(crate) use result_table::{Key as TableKey, PREVIEW_ROWS, TableView, full_rows};
#[allow(unused_imports)]
pub(crate) use sessions::{Command as SessionCommand, Picker};
#[allow(unused_imports)]
pub(crate) use sessions_row::{Facts as RowFacts, SessionRow, State as SessionState};
#[allow(unused_imports)]
pub(crate) use sessions_rows::rows as sessions_rows;
#[allow(unused_imports)]
pub(crate) use status::{ActivityState, StatusFacts, status_line};
#[allow(unused_imports)]
pub(crate) use toast::{TICK, Toast, row as toast_row};
#[allow(unused_imports)]
pub(crate) use transcript::{TranscriptFacts, rows as transcript_rows};
#[allow(unused_imports)]
pub(crate) use trust::ROWS as TRUST_ROWS;
#[allow(unused_imports)]
pub(crate) use trust::rows as trust_rows;

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
#[path = "blocks_wrap_tests.rs"]
mod blocks_wrap_tests;

#[cfg(test)]
#[path = "blocks_tests.rs"]
mod blocks_tests;

#[cfg(test)]
#[path = "cards_tests.rs"]
mod cards_tests;

#[cfg(test)]
#[path = "cards_panel_tests.rs"]
mod cards_panel_tests;

#[cfg(test)]
#[path = "cards_place_tests.rs"]
mod cards_place_tests;

#[cfg(test)]
#[path = "cards_diff_tests.rs"]
mod cards_diff_tests;

#[cfg(test)]
#[path = "composer_tests.rs"]
mod composer_tests;

#[cfg(test)]
#[path = "empty_fixture.rs"]
mod empty_fixture;

#[cfg(test)]
#[path = "empty_snapshots.rs"]
mod empty_snapshots;

#[cfg(test)]
#[path = "empty_tests.rs"]
mod empty_tests;

#[cfg(test)]
#[path = "help_coverage_tests.rs"]
mod help_coverage_tests;

#[cfg(test)]
#[path = "help_fixture.rs"]
mod help_fixture;

#[cfg(test)]
#[path = "help_tests.rs"]
mod help_tests;

#[cfg(test)]
#[path = "mode_picker_tests.rs"]
mod mode_picker_tests;

// The `@` list's pane: the rows it paints, their width and their ASCII form.
#[cfg(test)]
#[path = "mention_tests.rs"]
mod mention_tests;

#[cfg(test)]
#[path = "jobs_empty_tests.rs"]
mod jobs_empty_tests;

#[cfg(test)]
#[path = "jobs_fixture.rs"]
pub(crate) mod jobs_fixture;

#[cfg(test)]
#[path = "jobs_rows_tests.rs"]
mod jobs_rows_tests;

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod jobs_tests;

#[cfg(test)]
#[path = "notice_collapse_tests.rs"]
mod notice_collapse_tests;

#[cfg(test)]
#[path = "palette_fixture.rs"]
mod palette_fixture;

#[cfg(test)]
#[path = "palette_match_tests.rs"]
mod palette_match_tests;

#[cfg(test)]
#[path = "palette_order_tests.rs"]
mod palette_order_tests;

#[cfg(test)]
#[path = "palette_tests.rs"]
mod palette_tests;

#[cfg(test)]
mod result_fixture;

#[cfg(test)]
mod result_tests;

#[cfg(test)]
mod result_view_tests;

#[cfg(test)]
#[path = "sessions_fixture.rs"]
mod sessions_fixture;

#[cfg(test)]
#[path = "sessions_pane_tests.rs"]
mod sessions_pane_tests;

#[cfg(test)]
#[path = "sessions_rows_tests.rs"]
mod sessions_rows_tests;

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod sessions_tests;

#[cfg(test)]
#[path = "status_fixture.rs"]
mod status_fixture;

#[cfg(test)]
#[path = "status_fit_tests.rs"]
mod status_fit_tests;

#[cfg(test)]
#[path = "status_meter_tests.rs"]
mod status_meter_tests;

#[cfg(test)]
#[path = "status_tests.rs"]
mod status_tests;

#[cfg(test)]
#[path = "toast_tests.rs"]
mod toast_tests;

#[cfg(test)]
#[path = "transcript_fixture.rs"]
mod transcript_fixture;

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod transcript_tests;

#[cfg(test)]
#[path = "transcript_focus_tests.rs"]
mod transcript_focus_tests;

#[cfg(test)]
#[path = "transcript_rhythm_tests.rs"]
mod transcript_rhythm_tests;

#[cfg(test)]
#[path = "transcript_steps_fixture.rs"]
mod transcript_steps_fixture;

#[cfg(test)]
#[path = "transcript_detail_tests.rs"]
mod transcript_detail_tests;

#[cfg(test)]
#[path = "detail_hint_tests.rs"]
mod detail_hint_tests;

#[cfg(test)]
#[path = "welcome_suggestions_tests.rs"]
mod welcome_suggestions_tests;
