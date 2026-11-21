//! The picker's pane as a box: it fits the cells it is given at every width, it
//! cuts a title rather than spilling, it says nothing when the filter leaves
//! nothing, and it reads the same with colour off and in the ASCII glyph set.

use super::composer::Row;
use super::sessions::{NO_MATCH, Picker};
use super::sessions_fixture::{DONE, OPEN, RECORD, pane, picker};
use super::sessions_rows;
use crate::front::tui::theme::{Charset, Scheme, form};
use crate::interactive::tui::wrap::cell_width;

/// A filter that matches nothing says so, and draws no session row.
#[test]
fn a_filter_that_matches_nothing_says_so() {
    let mut picker = picker();
    for character in "zzzz".chars() {
        picker.type_char(character);
    }
    let rows = pane(&picker, 100, 30);
    assert!(
        rows.iter().any(|row| row.contains(NO_MATCH)),
        "the pane says the filter matched nothing: {rows:?}"
    );
    assert!(
        !rows
            .iter()
            .any(|row| row.contains(OPEN) || row.contains(DONE)),
        "and draws no session row: {rows:?}"
    );
}

/// No row of the pane outgrows the cells it was given, at any width a terminal
/// can be, for any filter.
#[test]
fn no_row_is_wider_than_the_pane() {
    for query in ["", "orders", "zzzz"] {
        let mut filtered = picker();
        for character in query.chars() {
            filtered.type_char(character);
        }
        for (width, height) in [(100, 30), (80, 24), (60, 12), (24, 8)] {
            let rows = sessions_rows::rows(&filtered, width, height);
            assert_eq!(
                rows.len(),
                height,
                "{query:?} at {width}x{height} is a full pane"
            );
            for row in &rows {
                let cells = cell_width(&row.text());
                assert!(
                    cells <= width,
                    "{:?} is {cells} cells wide at {width}: {query:?}",
                    row.text()
                );
            }
        }
    }
}

/// A pane too narrow for every column keeps the mark and cuts the title with an
/// ellipsis rather than spilling over the edge.
#[test]
fn a_narrow_pane_keeps_the_mark_and_cuts_the_title() {
    let rows = pane(&picker(), 24, 8);
    let record = rows[3].as_str();
    assert!(record.contains(RECORD), "the mark survives: {record:?}");
    assert!(
        record.contains('…'),
        "and the title is cut, not spilled: {record:?}"
    );
}

/// With colour off the pane says the same words: no run carries a hue, so the
/// mark and the words are the whole row.
#[test]
fn colour_off_paints_the_same_words() {
    let rows = sessions_rows::rows(&picker(), 100, 30);
    let mut words = Vec::new();
    for row in &rows {
        let mut line = String::new();
        for run in &row.runs {
            let style = form(run.token, Scheme::NoColour).style();
            assert!(
                style.fg.is_none(),
                "colour off paints no hue at all: {:?}",
                run.token
            );
            line.push_str(&run.text);
        }
        words.push(line);
    }
    assert_eq!(
        words,
        rows.iter().map(Row::text).collect::<Vec<_>>(),
        "colour is never the only thing a row says"
    );
    assert!(
        words.iter().any(|row| row.contains(RECORD)),
        "the record's mark is a glyph, not a hue"
    );
    assert!(
        words.iter().any(|row| row.contains("(this session)")),
        "and the tag is a word"
    );
}

/// `--ascii` reaches the pane: the same words, in the seven-bit glyph set.
#[test]
fn the_ascii_pane_says_the_same_things() {
    let picker = Picker {
        charset: Charset::Ascii,
        ..picker()
    };
    let rows = pane(&picker, 100, 30);
    assert!(rows[0].starts_with(super::sessions::TITLE), "{:?}", rows[0]);
    assert!(
        rows[1].starts_with("> *"),
        "the selection and the open run fall back: {:?}",
        rows[1]
    );
    assert!(
        rows[2].starts_with("  [ok]"),
        "a finished run falls back: {:?}",
        rows[2]
    );
    assert!(
        rows[3].starts_with("  o"),
        "a record with no run falls back: {:?}",
        rows[3]
    );
    let last = rows.last().expect("the pane has a bottom border");
    assert!(
        last.starts_with("[enter] resume - d delete - Esc close"),
        "{last:?}"
    );
}
