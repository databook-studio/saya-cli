//! The preview of `02` §3.3: five rows, numbers, NULL, the columns that fit,
//! and the one line a refused capture leaves.

use serde_json::json;

use super::result_fixture as fixture;
use super::result_rows;
use crate::agent::tools::CaptureRefusalReason;
use crate::front::tui::theme::{Scheme, Token, style};

/// Six rows are previewed as five: the sixth waits behind `Enter`.
#[test]
fn a_preview_shows_five_rows_of_six() {
    let shown = fixture::rendered(&result_rows(&fixture::orders(6), &fixture::facts(), 76));
    let table: Vec<&String> = shown.iter().filter(|row| row.starts_with("  │")).collect();
    assert_eq!(table.len(), 6, "the column names and five rows: {shown:?}");
    assert!(
        !table.iter().any(|row| row.contains('6')),
        "the sixth row waits for the full view: {shown:?}"
    );
}

/// A twelve-column result at `02` §3.3's 80 columns: the first columns that
/// fit, and the count of the ones that do not.
#[test]
fn a_twelve_column_result_counts_the_columns_it_cut() {
    let shown = fixture::rendered(&result_rows(&fixture::wide(12), &fixture::facts(), 76));
    assert_eq!(
        shown.last().map(String::as_str),
        Some("  +6 cols"),
        "the preview says how many columns did not fit: {shown:?}"
    );
    let header = shown
        .iter()
        .find(|row| row.starts_with("  │"))
        .expect("a header row");
    assert!(
        header.contains("column_06"),
        "the columns that fit show: {header:?}"
    );
    assert!(
        !header.contains("column_07"),
        "the columns that do not fit are counted, not cut: {header:?}"
    );
}

/// A NULL is an absence, not a value: it takes the `dim` token.
#[test]
fn a_null_cell_is_painted_dim() {
    let preview = fixture::result(&["id", "note"], vec![json!([1, null])]);
    let rows = result_rows(&preview, &fixture::facts(), 76);
    let null = rows
        .iter()
        .flat_map(|row| &row.spans)
        .find(|span| span.content.trim() == "NULL")
        .expect("the NULL cell is painted");
    assert_eq!(
        null.style,
        style(Token::Dim, Scheme::Dark),
        "a NULL takes the dim token: {:?}",
        null.content
    );
}

/// A number is right-aligned in its column, a word is not.
#[test]
fn numbers_are_right_aligned() {
    let numbers = fixture::rendered(&result_rows(
        &fixture::result(&["n"], vec![json!([1]), json!([22]), json!([333])]),
        &fixture::facts(),
        76,
    ));
    assert_eq!(&numbers[3..], ["  │   1 │", "  │  22 │", "  │ 333 │"]);
    let words = fixture::rendered(&result_rows(
        &fixture::result(&["name"], vec![json!(["a"]), json!(["bb"])]),
        &fixture::facts(),
        76,
    ));
    assert_eq!(&words[3..], ["  │ a    │", "  │ bb   │"]);
}

/// A refused capture draws no row at all: one dim line says why, in the words
/// `02` §3.3 fixes.
#[test]
fn a_refused_capture_shows_no_rows_and_says_why() {
    let preview = fixture::refused(CaptureRefusalReason::ModelViewTruncated);
    let rows = result_rows(&preview, &fixture::facts(), 76);
    let shown = fixture::rendered(&rows);
    assert_eq!(shown.len(), 2, "the query and one line: {shown:?}");
    assert!(
        !shown.iter().any(|row| row.contains('│')),
        "a refused capture draws no rows: {shown:?}"
    );
    assert_eq!(
        shown[1], "  model saw a truncated result; rerun with /sql to see rows",
        "the refusal line is verbatim: {shown:?}"
    );
}

/// Nothing the preview paints is wider than the pane it is painted in.
#[test]
fn nothing_the_preview_paints_exceeds_the_width() {
    for width in [100, 80, 60] {
        for preview in [
            fixture::orders(6),
            fixture::wide(12),
            fixture::result(&["id", "note"], vec![json!([1, null])]),
        ] {
            let rows = result_rows(&preview, &fixture::facts(), width - 4);
            assert!(
                rows.iter()
                    .any(|row| fixture::words(row).starts_with("  │")),
                "the preview paints a table at {width}: {:?}",
                fixture::rendered(&rows)
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
        let refused = fixture::rendered(&result_rows(
            &fixture::refused(CaptureRefusalReason::OverBudget),
            &fixture::facts(),
            width - 4,
        ));
        assert!(refused.iter().all(|row| row.chars().count() <= width));
    }
}
