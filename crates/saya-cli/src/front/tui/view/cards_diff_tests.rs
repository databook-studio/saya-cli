//! The approval panel with a change in its body (UI5): the hunk under the path,
//! the cap that says what it is not showing, `v` opening the rest, and the size
//! row a change that cannot be painted is stated as.
//!
//! The panel's facts are the ones UI3 pinned — the path, where it lands and the
//! answers — and every test here runs beside them, never instead of them.

use super::cards::{BODY_ROWS, approval};
use super::cards_diff::Ink;
use super::cards_fixture::{
    FOLDER, SCREEN_ROWS, binary_change_question, change_question, long_change_question,
    shell_question, write_question,
};

/// The rows' own text, in order.
fn texts(card: &super::cards::Card, full: bool, room: usize) -> Vec<String> {
    card.shown_rows(full, room)
        .iter()
        .map(|row| row.text().to_owned())
        .collect()
}

/// The row that states `text`, and its meaning.
fn row_of(card: &super::cards::Card, text: &str, full: bool, room: usize) -> (usize, Ink) {
    card.shown_rows(full, room)
        .iter()
        .enumerate()
        .find(|(_, row)| row.text() == text)
        .map(|(index, row)| (index, row.ink))
        .unwrap_or_else(|| panic!("no row states {text:?}: {:?}", card.shown_rows(full, room)))
}

#[test]
fn an_approval_with_a_change_states_the_hunk_under_the_path() {
    let card = approval(&change_question(), FOLDER, SCREEN_ROWS);

    assert_eq!(
        card.target(),
        ["src/pricing.py"],
        "the call's own rows are exactly as UI3 put them"
    );
    assert_eq!(
        card.change().first().map(String::as_str),
        Some("@@ -1,1 +1,1 @@"),
        "the change opens with its hunk header: {:?}",
        card.change()
    );

    let (path, _) = row_of(&card, "src/pricing.py", true, 90);
    let (header, header_ink) = row_of(&card, "@@ -1,1 +1,1 @@", true, 90);
    let (gone, gone_ink) = row_of(&card, "-    return total - discount - discount", true, 90);
    let (written, written_ink) = row_of(&card, "+    return total - discount", true, 90);

    assert!(path < header, "the change sits under the path it changes");
    assert!(
        header < gone && gone < written,
        "a hunk reads top to bottom"
    );
    assert_eq!(header_ink, Ink::Context, "a hunk header is where, not what");
    assert_eq!(gone_ink, Ink::Removed, "the line taken away is a minus row");
    assert_eq!(written_ink, Ink::Added, "the line written is a plus row");
}

#[test]
fn the_signs_are_in_the_rows_not_only_in_the_colour() {
    let card = approval(&change_question(), FOLDER, SCREEN_ROWS);

    for row in card.shown_rows(true, 90) {
        match row.ink {
            Ink::Added => assert!(
                row.text().starts_with('+'),
                "an added row carries its own sign: {row:?}"
            ),
            Ink::Removed => assert!(
                row.text().starts_with('-'),
                "a removed row carries its own sign: {row:?}"
            ),
            _ => {}
        }
    }
}

#[test]
fn the_folded_body_states_the_lines_it_is_not_showing() {
    let card = approval(&long_change_question(), FOLDER, SCREEN_ROWS);
    let folded = card.shown_rows(false, 90);

    assert_eq!(
        folded.len(),
        BODY_ROWS,
        "80x24 shows at most the panel's own six rows"
    );
    let last = folded.last().expect("the body has rows");
    assert_eq!(
        last.ink,
        Ink::Withheld,
        "the cut is stated as a row of its own: {folded:?}"
    );
    assert_eq!(
        last.text(),
        "+9 more lines",
        "and it says how many the cap left out: {folded:?}"
    );
    assert_eq!(
        card.withheld(90),
        9,
        "`v` opens exactly what the count states"
    );
}

#[test]
fn view_opens_the_whole_bounded_change() {
    let card = approval(&long_change_question(), FOLDER, SCREEN_ROWS);
    let full = card.shown_rows(true, 90);

    assert_eq!(
        full.len(),
        1 + card.change().len(),
        "the path, then every row of the change, bounded by the diff's own cap"
    );
    assert!(
        full.iter().all(|row| row.ink != Ink::Withheld),
        "nothing is withheld once `v` has opened it: {full:?}"
    );
    assert!(
        full.iter().any(|row| row.text().contains("row 11")),
        "the last row of the change is on the panel: {full:?}"
    );
}

#[test]
fn a_call_that_changes_no_file_states_no_hunk() {
    let card = approval(&shell_question(), FOLDER, SCREEN_ROWS);

    assert!(
        card.change().is_empty(),
        "a shell command changes no file text: {:?}",
        card.change()
    );
    assert_eq!(
        texts(&card, true, 90),
        card.target(),
        "and the body is exactly the command, as UI3 settled it"
    );
}

#[test]
fn a_change_that_is_not_text_is_stated_as_its_size() {
    let card = approval(&binary_change_question(), FOLDER, SCREEN_ROWS);
    let change = card.change();

    assert_eq!(change.len(), 1, "one size row, never bytes: {change:?}");
    assert!(
        change[0].contains("binary") && change[0].contains("7 bytes"),
        "the row states the kind and the size: {change:?}"
    );
    assert!(
        card.shown_rows(true, 90)
            .iter()
            .all(|row| !matches!(row.ink, Ink::Added | Ink::Removed)),
        "no line of a binary change is painted as a line"
    );
}

#[test]
fn a_write_is_never_a_second_surface_of_its_own() {
    let plain = approval(&write_question(), FOLDER, SCREEN_ROWS);
    let changed = approval(&change_question(), FOLDER, SCREEN_ROWS);

    assert_eq!(
        plain.title(),
        changed.title(),
        "the title is not restated for a change"
    );
    assert_eq!(
        plain.choices(),
        changed.choices(),
        "and no answer is added, moved or renamed — the decision is unchanged"
    );
    assert_eq!(
        plain.target(),
        changed.target(),
        "the path the operator answers for is the same row it was"
    );
}
