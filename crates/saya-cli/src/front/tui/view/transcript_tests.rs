//! The transcript pane: `02` §2's turn as rows, a turn's blocks, and the live
//! edge.
//!
//! Every snapshot asserts the whole painted rows — words, indents and the air a
//! right-aligned hint sits behind — because a transcript's look *is* the rows.

use super::transcript_fixture as fixture;
use super::transcript_fixture::{ELAPSED, rendered};
use super::{transcript, transcript_rows};

/// The turn of `02` §2 at 100x30, row for row.
#[test]
fn the_hundred_column_turn_reads_as_02_section_2() {
    let rows = transcript_rows(
        &[fixture::database_turn(true)],
        &fixture::facts(ELAPSED),
        100,
        30,
    );
    assert_eq!(
        rendered(&rows),
        vec![
            "  ❯ how many orders did we get since December?",
            "",
            "  ✓ 4 steps · read schema, 3 queries · 14s                                                 ▸ Enter",
            "  110 orders since December 2025 (2025-12-01 through 2026-01-01, the latest date in the data).",
            "",
            "  Notes on interpretation:",
            "  • The data contains only one December (2025), so \"since December\" = from 2025-12-01 onward,",
            "    which includes the 5 orders on 2026-01-01. December 2025 alone would be 105.",
            "  • I counted all orders regardless of status (completed, pending, refunded) — 269 completed, 144",
            "    pending, 147 refunded overall. Say the word if you want only completed.",
            "",
            "  SELECT COUNT(*) AS orders_since_december FROM orders",
            "  result · demo · 1 row",
        ]
    );
}

/// The same turn at 80x24: the answer wraps where the content column ends.
#[test]
fn the_eighty_column_turn_reads_as_02_section_2() {
    let rows = transcript_rows(
        &[fixture::database_turn(true)],
        &fixture::facts(ELAPSED),
        80,
        24,
    );
    assert_eq!(
        rendered(&rows),
        vec![
            "  ❯ how many orders did we get since December?",
            "",
            "  ✓ 4 steps · read schema, 3 queries · 14s                             ▸ Enter",
            "  110 orders since December 2025 (2025-12-01 through 2026-01-01, the latest",
            "  date in the data).",
            "",
            "  Notes on interpretation:",
            "  • The data contains only one December (2025), so \"since December\" = from",
            "    2025-12-01 onward, which includes the 5 orders on 2026-01-01. December",
            "    2025 alone would be 105.",
            "  • I counted all orders regardless of status (completed, pending, refunded)",
            "    — 269 completed, 144 pending, 147 refunded overall. Say the word if you",
            "    want only completed.",
            "",
            "  SELECT COUNT(*) AS orders_since_december FROM orders",
            "  result · demo · 1 row",
        ]
    );
}

/// `02` §3: the blocks carry no role label — no `YOU`, no `SAYA`, no
/// `ACTIVITY`. Only a glyph and the operator's own words open a turn.
#[test]
fn no_row_carries_a_role_label() {
    let labels = ["YOU", "SAYA", "ACTIVITY"];
    for width in [100, 80] {
        for turn in [
            fixture::database_turn(true),
            fixture::database_turn(false),
            fixture::live_turn(),
            fixture::failed_turn(),
        ] {
            let rows = transcript_rows(&[turn], &fixture::facts(ELAPSED), width, 24);
            for row in rendered(&rows) {
                for label in labels {
                    assert!(
                        !row.contains(label),
                        "no row carries the {label} label: {row:?}"
                    );
                }
            }
        }
    }
}

/// While the turn runs the group shows its steps: the last four, the older
/// ones counted, each step under the block's gutter.
#[test]
fn a_running_turn_shows_its_live_steps() {
    let rows = transcript_rows(&[fixture::live_turn()], &fixture::facts(ELAPSED), 100, 30);
    assert_eq!(
        rendered(&rows),
        vec![
            "  ❯ how many orders did we get since December?",
            "",
            "  ● Working · 14s · 3 earlier steps                                                        ▸ Enter",
            "    ✓ Query demo · 1 row",
            "    ✓ Query demo · 2 rows",
            "    ✓ Query demo · 5 rows",
            "    ● Query demo",
        ]
    );
}

/// A settled turn folds to one line, and its steps are gone: `02` §3.2's
/// `✓ 4 steps · read schema, 3 queries · 14s`.
#[test]
fn a_settled_turn_folds_to_its_one_summary_line() {
    let live = rendered(&transcript_rows(
        &[fixture::database_turn(false)],
        &fixture::facts(ELAPSED),
        100,
        30,
    ));
    assert!(
        !live
            .iter()
            .any(|row| row.trim_start().starts_with("✓ 4 steps")),
        "a running block shows its steps, not a summary: {live:?}"
    );
    let folded = rendered(&transcript_rows(
        &[fixture::database_turn(true)],
        &fixture::facts(ELAPSED),
        100,
        30,
    ));
    assert!(!folded.iter().any(|row| row.contains("Query demo")));
    assert!(!folded.iter().any(|row| row.contains("Read schema")));
    assert!(
        folded
            .iter()
            .any(|row| row.starts_with("  ✓ 4 steps · read schema, 3 queries · 14s")),
        "the block folds to its one line: {folded:?}"
    );
}

/// A failed step never folds: what went wrong is the part to read, however
/// the block was left marked.
#[test]
fn a_failed_step_stays_unfolded() {
    let rows = transcript_rows(&[fixture::failed_turn()], &fixture::facts(ELAPSED), 100, 30);
    assert_eq!(
        rendered(&rows),
        vec![
            "  ❯ how many orders did we get since December?",
            "",
            "  ● Working · 14s",
            "    ✓ Read schema",
            "    ✗ Query demo — failed to complete: connection refused",
        ]
    );
}

/// The newest turn sits at the bottom: 60 rows of content in 24 rows of pane
/// shows the last 24.
#[test]
fn the_live_edge_shows_the_last_rows() {
    let rows = transcript_rows(
        &[fixture::answer_turn(60)],
        &fixture::facts(ELAPSED),
        100,
        24,
    );
    let shown = rendered(&rows);
    assert_eq!(shown.len(), 24);
    assert_eq!(shown.first().map(String::as_str), Some("  line 37"));
    assert_eq!(shown.last().map(String::as_str), Some("  line 60"));
}

/// Nothing a turn paints is wider than the pane it is painted in.
#[test]
fn nothing_exceeds_the_width() {
    for width in [100, 80, 60] {
        for turn in [
            fixture::database_turn(true),
            fixture::database_turn(false),
            fixture::live_turn(),
            fixture::failed_turn(),
        ] {
            let rows = transcript_rows(&[turn], &fixture::facts(ELAPSED), width, 30);
            assert!(!rows.is_empty());
            for row in &rows {
                let cells = fixture::cells(row);
                assert!(
                    cells <= width,
                    "a row of {cells} cells does not fit {width}: {:?}",
                    fixture::words(row)
                );
            }
        }
    }
    // The pane's own function, not only the re-export: the live edge keeps the
    // same width.
    let rows = transcript::rows(
        &[fixture::answer_turn(60)],
        &fixture::facts(ELAPSED),
        100,
        24,
    );
    for row in &rows {
        assert!(fixture::cells(row) <= 100);
    }
}
