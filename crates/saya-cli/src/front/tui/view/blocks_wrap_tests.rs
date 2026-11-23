//! The session's own line on a narrow terminal: a long notice wraps, and is
//! never cut.
//!
//! The finding this test pins: an operator's 50-column terminal cut a notice
//! mid-word, because the renderer took a logical line through the truncating
//! row painter. A sentence the session says is a sentence the operator must be
//! able to read, so a notice that does not fit its room takes another row.

use super::blocks_fixture as fixture;
use crate::front::view::{Entry, Notice};

/// The sentence `/undo` answers with, at the width that cut it.
const LONG: &str = "`/undo` and `/rewind` restore a job run's checkpoints, and this session runs no job: \
     its recorded file edits are the per-edit store, where `/recovery list` shows them and \
     `/recovery restore <id>` puts one back";

/// The columns the operator's terminal had.
const NARROW: usize = 50;

/// A notice wider than the window takes as many rows as it needs, and every
/// word of it is painted: nothing is replaced by an ellipsis.
#[test]
fn a_long_notice_wraps_instead_of_being_cut() {
    let rows = painted_at(LONG, NARROW);
    let whole: String = rows
        .iter()
        .map(|row| row.trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !whole.contains('…'),
        "a notice is never cut with an ellipsis: {rows:#?}"
    );
    for word in ["checkpoints", "per-edit", "puts one back"] {
        assert!(
            whole.contains(word),
            "the whole sentence is painted, and `{word}` is in it: {rows:#?}"
        );
    }
    assert!(
        rows.len() > 1,
        "a notice that does not fit takes more than one row: {rows:#?}"
    );
}

/// Every row is inside the window: wrapping is not painting past the edge.
#[test]
fn every_row_of_a_wrapped_notice_fits_the_window() {
    let rows = painted_at(LONG, NARROW);
    for row in &rows {
        assert!(
            fixture::cells_of(row) <= NARROW,
            "a row paints inside {NARROW} columns: {row:?} is {} cells",
            fixture::cells_of(row)
        );
    }
}

/// A notice that already carries lines keeps them, and wraps each: `/help`'s
/// listing is several lines, not one run-on paragraph.
#[test]
fn a_multi_line_notice_keeps_its_own_lines() {
    let message = format!("first line\n{LONG}\nlast line");
    let rows = painted_at(&message, NARROW);
    let whole: String = rows
        .iter()
        .map(|row| row.trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(whole.contains("first line"), "{rows:#?}");
    assert!(whole.contains("last line"), "{rows:#?}");
    assert!(whole.contains("puts one back"), "{rows:#?}");
}

/// The notice `message`, painted at `width`.
fn painted_at(message: &str, width: usize) -> Vec<String> {
    fixture::painted(
        Entry::Notice(Notice {
            message: message.to_owned(),
        }),
        width,
    )
}
