//! The turn's work blocks (`02` §3.4-§3.6) and the session's own line (§3.10):
//! each as the rows a terminal paints, called from the turn painter.
//!
//! Every snapshot asserts the whole painted block — words, indents, the air a
//! right-aligned key sits behind — because a transcript's look *is* the rows.

use super::blocks_fixture as fixture;
use crate::front::tui::theme::Scheme;
use crate::front::view::Entry;

/// The edit of `02` §3.4 at 100 columns, row for row: its file and counts, then
/// the diff with the `+`/`-` gutter.
#[test]
fn the_edit_block_reads_as_02_section_3_4() {
    assert_eq!(
        fixture::painted(Entry::Edit(fixture::edit()), 100),
        vec![
            "  ✎ Edit pricing.py  +1 −1                                                                 ▸ Enter",
            "      14 │     discount = total * percent_off // 100",
            "      15 │-    return total - discount - discount",
            "      15 │+    return total - discount",
        ]
    );
}

/// A diff longer than the block shows is capped, and the rows it withholds are
/// counted in one `+N more lines` row.
#[test]
fn a_long_diff_caps_and_counts_the_rest() {
    let rows = fixture::painted(Entry::Edit(fixture::long_edit()), 100);
    assert_eq!(
        rows,
        vec![
            "  ✎ Edit pricing.py  +3 −1                                                                 ▸ Enter",
            "      1 │ def total(orders):",
            "      2 │-    return sum(o.total for o in orders) - 10",
            "      2 │+    return sum(o.total for o in orders)",
            "      +3 more lines",
        ]
    );
}

/// The command of `02` §3.5: `$` and the command, its status and duration
/// right-aligned at the content edge.
#[test]
fn the_command_block_reads_as_02_section_3_5() {
    assert_eq!(
        fixture::painted(Entry::Command(fixture::command()), 100),
        vec![
            "  $ python3 -m unittest                                                          ✓ finished · 0.4s"
        ]
    );
}

/// The command of `02` §3.5 is shown relative to the workspace: the `cd` that
/// only exists to reach the root is stripped (P9).
#[test]
fn a_command_is_shown_relative_to_the_workspace() {
    assert_eq!(
        fixture::painted(Entry::Command(fixture::command_under_root()), 100),
        vec![
            "  $ python3 -m unittest                                                          ✓ finished · 0.4s"
        ]
    );
}

/// The helper of `02` §3.6: its id, its brief in its own words, how many steps
/// it took and how it ended.
#[test]
fn the_helper_block_reads_as_02_section_3_6() {
    assert_eq!(
        fixture::painted(Entry::Helper(fixture::helper()), 100),
        vec![
            "  ◆ helper explore-1 · \"find where pricing is tested\" · 3 steps · ✓ done                   ▸ Enter"
        ]
    );
}

/// The session's own line of `02` §3.10: one compact dim row, no echo.
#[test]
fn the_system_line_is_one_row() {
    assert_eq!(
        fixture::painted(Entry::Notice(fixture::system()), 100),
        vec!["  mode: accept-edits"]
    );
}

/// The question of `02` §3.9: the ask, its numbered options and the cue that
/// the answer is typed into the composer.
#[test]
fn the_question_block_reads_as_02_section_3_9() {
    assert_eq!(
        fixture::painted(Entry::Question(fixture::question()), 100),
        vec![
            "  ? Which revenue do you mean?",
            "    1. Gross order value (orders.total)",
            "    2. Net of refunds (orders.total − refunds.amount)",
            "    3. Recognised revenue (ledger.revenue)",
            "    Type a number or your own answer.",
        ]
    );
}

/// Every block of `02` §3.4-§3.10 is painted, and no row of any of them is
/// wider than the pane at 100, 80 or 60 columns.
#[test]
fn nothing_exceeds_the_width() {
    for width in [100, 80, 60] {
        for (name, entry) in fixture::every_block() {
            let rows = fixture::painted(entry, width);
            assert!(!rows.is_empty(), "the {name} block paints rows");
            for row in rows {
                let cells = fixture::cells_of(&row);
                assert!(
                    cells <= width,
                    "a {name} row of {cells} cells does not fit {width}: {row:?}"
                );
            }
        }
    }
}

/// Colour off is the same words: every block's meaning is carried by its glyph,
/// its sign or its words, never by a hue (`02` §12).
#[test]
fn colour_off_renders_the_same_words() {
    for (name, entry) in fixture::every_block() {
        for width in [100, 80] {
            let dark = fixture::painted_in(entry.clone(), width, Scheme::Dark);
            let bare = fixture::painted_in(entry.clone(), width, Scheme::NoColour);
            assert_eq!(dark, bare, "the {name} block at {width} reads the same");
        }
    }
}
