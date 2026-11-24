//! The mode picker's pane, read back as words: the six rows of `02` §8 with
//! their one line each, the row the mode in force is marked on, the keys the
//! pane reads, and the words a change says.
//!
//! The words below are this test's own copy of `02` §8, so a picker that
//! reworded a row would fail here instead of agreeing with itself.

use saya_agent::PermissionMode;

use super::mode_picker::{self, ModePicker};
use crate::front::tui::theme::Charset;
use crate::interactive::session::permissions::mode::name;

/// `02` §8's six rows: the mode, and the one line it means, verbatim.
const SIX: [(PermissionMode, &str); 6] = [
    (
        PermissionMode::Ask,
        "every edit, command, fetch and query asks first",
    ),
    (
        PermissionMode::AcceptEdits,
        "file edits run; commands, fetches, queries ask",
    ),
    (PermissionMode::Plan, "read and plan only; writes refuse"),
    (
        PermissionMode::Bypass,
        "nothing asks (allowed by [permissions])",
    ),
    (
        PermissionMode::ReadOnly,
        "no writes, no host commands; nothing asks",
    ),
    (
        PermissionMode::Never,
        "no tool needs approval because none are offered",
    ),
];

/// The pane's rows as text, at `width × height`, with the mode in force set.
fn pane(current: PermissionMode, width: usize, height: usize) -> Vec<String> {
    pane_in(current, width, height, Charset::Utf8)
}

/// The pane's rows as text in one glyph set.
fn pane_in(current: PermissionMode, width: usize, height: usize, charset: Charset) -> Vec<String> {
    mode_picker::rows(&ModePicker::on(current), width, height, charset)
        .iter()
        .map(|row| row.text())
        .collect()
}

/// The row that says `words`, wherever the pane put it.
fn row_saying<'a>(rows: &'a [String], words: &str) -> &'a String {
    rows.iter()
        .find(|row| row.contains(words))
        .unwrap_or_else(|| panic!("no row says {words:?} in {rows:#?}"))
}

#[test]
fn the_pane_lists_the_six_modes_with_their_words_at_a_hundred_columns() {
    let rows = pane(PermissionMode::Ask, 100, 8);

    assert_eq!(rows.len(), 8, "the pane is exactly its height: {rows:#?}");
    assert!(
        rows[0].contains(mode_picker::TITLE),
        "the top border names the pane: {}",
        rows[0]
    );
    for (mode, words) in SIX {
        let row = row_saying(&rows, words);
        assert!(
            row.contains(name(mode)),
            "the row carrying {:?} names its mode: {row}",
            words
        );
    }
}

#[test]
fn the_same_six_rows_fit_at_eighty_columns() {
    let rows = pane(PermissionMode::Ask, 80, 8);

    assert_eq!(rows.len(), 8, "the pane is exactly its height: {rows:#?}");
    for (mode, words) in SIX {
        let row = row_saying(&rows, words);
        assert!(
            row.contains(name(mode)),
            "at 80 columns the row still names {:?}: {row}",
            name(mode)
        );
        assert!(
            row.chars().count() <= 80,
            "no row is wider than the pane: {row}"
        );
    }
}

#[test]
fn the_mode_in_force_is_marked_and_already_selected() {
    let picker = ModePicker::on(PermissionMode::Plan);
    assert_eq!(
        picker.chosen(),
        PermissionMode::Plan,
        "the picker opens on the mode in force"
    );

    let rows = pane(PermissionMode::Plan, 100, 8);
    for (mode, words) in SIX {
        let row = row_saying(&rows, words);
        let marked = row.contains('●');
        assert_eq!(
            marked,
            mode == PermissionMode::Plan,
            "exactly the row of the mode in force carries the mark: {row}"
        );
    }
}

#[test]
fn the_selection_stops_at_both_ends() {
    let mut picker = ModePicker::on(PermissionMode::Ask);
    picker.up();
    assert_eq!(
        picker.chosen(),
        PermissionMode::Ask,
        "up on the first row stays on it"
    );

    for _ in 0..9 {
        picker.down();
    }
    assert_eq!(
        picker.chosen(),
        PermissionMode::Never,
        "down stops at the last row"
    );

    picker.up();
    assert_eq!(
        picker.chosen(),
        PermissionMode::ReadOnly,
        "up steps back one row from the end"
    );
}

#[test]
fn the_pane_names_the_keys_it_reads() {
    let rows = pane(PermissionMode::Ask, 100, 8);
    let bottom = rows.last().expect("the pane has a bottom border");
    assert!(
        bottom.contains("↑↓ choose · Enter set · Esc cancel"),
        "the bottom border is 02 §8's key row: {bottom}"
    );

    let ascii = pane_in(PermissionMode::Ask, 100, 8, Charset::Ascii);
    let bottom = ascii.last().expect("the pane has a bottom border");
    assert!(
        bottom.contains("Up/Down choose - Enter set - Esc cancel"),
        "the ASCII pane says the same keys: {bottom}"
    );
}

#[test]
fn no_row_is_wider_than_the_pane() {
    for width in [100, 80, 60, 24] {
        for height in [8, 6, 1] {
            let rows = mode_picker::rows(
                &ModePicker::on(PermissionMode::Ask),
                width,
                height,
                Charset::Utf8,
            );
            assert_eq!(rows.len(), height, "a {width}×{height} pane is its height");
            for row in &rows {
                let text = row.text();
                assert!(
                    text.chars().count() <= width.max(12),
                    "a {width}-column pane has no row past its edge: {text}"
                );
            }
        }
    }
}

#[test]
fn every_mode_has_its_one_line() {
    for (mode, words) in SIX {
        assert_eq!(
            mode_picker::words(mode),
            words,
            "the picker's line for {} is 02 §8's",
            name(mode)
        );
    }
}

#[test]
fn a_change_is_one_transcript_line_and_the_toast_of_section_four() {
    assert_eq!(
        mode_picker::line(PermissionMode::Plan),
        "mode: plan",
        "the transcript's line is 02 §3.10's"
    );
    assert_eq!(
        mode_picker::toast(PermissionMode::Plan),
        "mode → plan: write tools refuse until you leave plan",
        "the plan toast is 02 §4's sentence, whole"
    );
    for (mode, _) in SIX {
        let toast = mode_picker::toast(mode);
        let head = format!("mode → {}: ", name(mode));
        assert!(
            toast.starts_with(&head) && toast.len() > head.len(),
            "every mode's toast names the mode and what it means: {toast}"
        );
    }
}
