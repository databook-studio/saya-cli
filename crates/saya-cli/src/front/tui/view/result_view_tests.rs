//! The full table view: the column window its keys move, the pin that holds
//! the first column, and the rows a pane of a given size holds.

use super::result_fixture as fixture;
use super::result_table::{Key, TableView, full_rows};

/// `Ctrl+,` and `Ctrl+.` move the window one column at a time and stop at the
/// table's edges.
#[test]
fn the_column_keys_move_the_window_and_clamp() {
    let preview = fixture::wide(12);
    let mut view = TableView::default();
    let start = fixture::rendered(&full_rows(&preview, &view, &fixture::facts(), 40, 24));
    assert!(
        start[1].contains("column_01"),
        "the window starts at the first column: {start:?}"
    );

    view.apply(Key::ColumnRight, 12);
    view.apply(Key::ColumnRight, 12);
    assert_eq!(view.offset(), 2, "two presses move the window two columns");
    let moved = fixture::rendered(&full_rows(&preview, &view, &fixture::facts(), 40, 24));
    assert!(
        moved[1].contains("column_03"),
        "the window now starts at the third column: {moved:?}"
    );
    assert!(
        !moved[1].contains("column_01"),
        "the first column left the window: {moved:?}"
    );

    for _ in 0..20 {
        view.apply(Key::ColumnRight, 12);
    }
    assert_eq!(view.offset(), 11, "the window stops at the last column");
    let last = fixture::rendered(&full_rows(&preview, &view, &fixture::facts(), 40, 24));
    assert!(
        last.iter().any(|row| row.contains("column_12")),
        "the last column is reachable: {last:?}"
    );

    for _ in 0..20 {
        view.apply(Key::ColumnLeft, 12);
    }
    assert_eq!(view.offset(), 0, "the window stops at the first column");
}

/// `Ctrl+P` holds the first column while the rest scroll past it.
#[test]
fn a_pinned_first_column_stays_while_the_rest_scroll() {
    let preview = fixture::wide(12);
    let mut view = TableView::default();
    view.apply(Key::Pin, 12);
    view.apply(Key::ColumnRight, 12);
    view.apply(Key::ColumnRight, 12);
    let rows = fixture::rendered(&full_rows(&preview, &view, &fixture::facts(), 40, 24));
    assert!(
        rows[1].contains("column_01"),
        "the pinned column stays in the window: {rows:?}"
    );
    assert!(
        rows[1].contains("column_03"),
        "the columns after it scrolled: {rows:?}"
    );
}

/// The full view paints no row wider than the pane and no row a pane of
/// `height` cannot hold.
#[test]
fn the_full_view_fits_its_width_and_height() {
    let preview = fixture::wide(12);
    let mut view = TableView::default();
    view.apply(Key::Pin, 12);
    view.apply(Key::ColumnRight, 12);
    for width in [100, 80, 40] {
        for height in [24, 8, 2] {
            let rows = full_rows(&preview, &view, &fixture::facts(), width, height);
            assert!(!rows.is_empty());
            assert!(
                rows.len() <= height,
                "{} rows do not fit a pane of {height}",
                rows.len()
            );
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
}
