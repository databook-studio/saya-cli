//! The transcript with the block cursor on it (`02` §6): the `▸` the focused
//! entry carries, the inspector line `i` shows, and a failed group that stays
//! expanded however it was marked.
//!
//! The rows are asserted whole, because the cursor's whole effect is what the
//! rows look like: two cells of gutter traded for the mark, and one dim row
//! more when the inspector is on.

use super::transcript_fixture as fixture;
use super::transcript_fixture::{ELAPSED, rendered};
use super::transcript_rows;
use crate::front::tui::focus::EntryPath;

/// The `02` §2 turn at 100 columns, with the cursor on `at` and the inspector
/// in the state `inspector` names.
fn focused(at: EntryPath, inspector: bool) -> Vec<String> {
    let mut facts = fixture::facts(ELAPSED);
    facts.focus = Some(at);
    facts.inspector = inspector;
    rendered(&transcript_rows(
        &[fixture::database_turn(true)],
        &facts,
        100,
        30,
    ))
}

/// Nothing painted, the cursor nowhere: the rows as T5 paints them.
fn unfocused() -> Vec<String> {
    rendered(&transcript_rows(
        &[fixture::database_turn(true)],
        &fixture::facts(ELAPSED),
        100,
        30,
    ))
}

#[test]
fn the_focused_entry_carries_the_mark_at_a_hundred_columns() {
    let plain = unfocused();
    let marked = focused(EntryPath::new(0, 1), false);

    // The activity group is the turn's second entry, and §2 draws it third.
    let at = 2;
    assert!(
        plain[at].starts_with("  ✓ 4 steps"),
        "the unfocused group starts at the gutter: {:?}",
        plain[at]
    );
    // The mark takes the gutter, and the header's hint says what the selected
    // entry answers: the block's own key replaces the surface's, and the hint
    // stays right-aligned, so only the air behind it moves.
    let expected = format!(
        "▸ ✓ 4 steps · read schema, 3 queries · 14s{}Enter open",
        " ".repeat(46)
    );
    assert_eq!(
        marked[at], expected,
        "the focused row trades its first two cells for §6's mark"
    );
    assert_eq!(
        plain[at].chars().count(),
        marked[at].chars().count(),
        "and the row keeps the width it had"
    );
    assert_eq!(
        marked.iter().filter(|row| row.starts_with('▸')).count(),
        1,
        "exactly one row is marked: {marked:?}"
    );
    assert_eq!(marked.len(), plain.len(), "marking adds no row: {marked:?}");
    for (index, row) in marked.iter().enumerate() {
        if index != at {
            assert_eq!(*row, plain[index], "no other row moved: {row:?}");
        }
    }
}

#[test]
fn the_inspector_line_names_the_focused_results_provenance() {
    // The result preview is the turn's fourth entry, and §2's last block.
    let at = EntryPath::new(0, 3);
    let without = focused(at, false);
    let shown = focused(at, true);

    assert_eq!(
        shown.len(),
        without.len() + 1,
        "`i` adds exactly one row: {shown:?}"
    );
    assert_eq!(
        shown.last().map(String::as_str),
        Some("  demo · 1 row"),
        "the inspector names the connection and the rows it held: {shown:?}"
    );
    assert_eq!(
        &shown[..shown.len() - 1],
        &without[..],
        "and nothing else about the entry changes"
    );
}

#[test]
fn a_failed_focused_group_is_painted_expanded() {
    let mut facts = fixture::facts(ELAPSED);
    facts.focus = Some(EntryPath::new(0, 1));
    let rows = rendered(&transcript_rows(&[fixture::failed_turn()], &facts, 100, 30));
    let shown = rows.join("\n");

    assert!(
        rows.iter().any(|row| row.starts_with("▸ ✓ 2 steps")),
        "the failed group's header carries the mark: {rows:?}"
    );
    assert!(
        shown.contains("✗ Query demo: SELECT COUNT(*) AS orders_since_december FROM orders")
            && shown.contains("connection refused"),
        "and its steps stay open under it, the reason wrapped whole: {rows:?}"
    );
}
