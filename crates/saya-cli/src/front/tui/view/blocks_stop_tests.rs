//! The blocks that end a turn: the error (`02` §3.7) and the stops — budget,
//! pause, cancel (`02` §3.8).
//!
//! The budget's second row is the shared partial wording, so one test asserts
//! it character for character: the goldens and scripts read that sentence.

use saya_agent::BudgetDimension;

use super::blocks_fixture as fixture;
use crate::budget_wording::budget_partial_message;
use crate::front::view::Entry;

/// The error of `02` §3.7: its reason on the row the glyph opens, then what the
/// operator can still do — at the design's 100 columns and at its 80.
#[test]
fn the_error_block_reads_as_02_section_3_7() {
    let expected = vec![
        "  ✗ Provider error — rate limited (429). Retried 3 times.",
        "    Your message is kept: ↑ to edit, Enter to resend.",
    ];
    assert_eq!(
        fixture::painted(Entry::Error(fixture::error()), 100),
        expected,
        "at 100 columns"
    );
    assert_eq!(
        fixture::painted(Entry::Error(fixture::error()), 80),
        expected,
        "and at the design's 80"
    );
}

/// The budget stop of `02` §3.8 at 100 columns: the stop's name, the partial
/// wording and the limit that bound the turn.
#[test]
fn the_budget_stop_reads_as_02_section_3_8() {
    assert_eq!(
        fixture::painted(Entry::Stop(fixture::budget()), 100),
        vec![
            "  ■ Database budget reached",
            "    the run stopped on its elapsed-time budget; any answer it produced is partial or absent",
            "    Ask again to continue · limit: [limits.database] elapsed_seconds = 300",
        ]
    );
}

/// The budget's second row is `budget_partial_message` verbatim — the one
/// sentence the goldens and scripts read, asserted here character for
/// character.
#[test]
fn the_budget_sentence_is_verbatim() {
    let rows = fixture::painted(Entry::Stop(fixture::budget()), 100);
    assert_eq!(
        rows.get(1).map(String::as_str),
        Some(
            "    the run stopped on its elapsed-time budget; any answer it produced is partial or \
             absent"
        ),
        "the second row is the sentence `02` §3.8 fixes"
    );
    assert_eq!(
        budget_partial_message(BudgetDimension::ElapsedTime),
        "the run stopped on its elapsed-time budget; any answer it produced is partial or absent",
        "and it is the shared wording, not a second copy of it"
    );
}

/// The same block at 80 columns wraps the sentence rather than losing a word of
/// it.
#[test]
fn the_budget_stop_wraps_at_eighty_columns() {
    assert_eq!(
        fixture::painted(Entry::Stop(fixture::budget()), 80),
        vec![
            "  ■ Database budget reached",
            "    the run stopped on its elapsed-time budget; any answer it produced is",
            "    partial or absent",
            "    Ask again to continue · limit: [limits.database] elapsed_seconds = 300",
        ]
    );
}

/// The pause of `02` §3.8: the guard's own sentence and the keys that continue.
#[test]
fn the_pause_reads_as_02_section_3_8() {
    assert_eq!(
        fixture::painted(Entry::Stop(fixture::paused()), 100),
        vec![
            "  ⏸ Paused — no progress in 8 turns (limits.job.no_progress_turns).",
            "    Enter continue · type to redirect · /status for details",
        ]
    );
}

/// The cancel of `02` §3.8: one row, and nothing after it ran.
#[test]
fn the_cancel_reads_as_02_section_3_8() {
    assert_eq!(
        fixture::painted(Entry::Stop(fixture::cancelled()), 100),
        vec!["  ⊘ Stopped by you. Nothing after this point ran."]
    );
}

/// An error is never folded into the activity group and never cut: a long
/// reason wraps and keeps every word.
#[test]
fn a_long_error_reason_wraps_instead_of_being_cut() {
    let long = crate::front::view::Error {
        message: "Provider error — the provider refused the request after three retries, so \
                  this turn stopped before it could answer."
            .to_owned(),
    };
    let rows = fixture::painted(Entry::Error(long), 60);
    assert!(rows.len() > 2, "the reason wraps: {rows:?}");
    assert!(
        rows[0].starts_with("  ✗ Provider error — the provider refused"),
        "the reason opens the block: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.trim_end().ends_with("could answer.")),
        "the reason stays whole: {rows:?}"
    );
    for row in &rows {
        assert!(
            !row.contains('…'),
            "a reason is never cut with an ellipsis: {row:?}"
        );
    }
}
