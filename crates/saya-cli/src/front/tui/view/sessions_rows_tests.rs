//! The picker's pane, read as the rows a terminal would paint: the state marks,
//! the words that carry each state, and the facts an operator chooses by.

use super::sessions::{FILTER_HINT, Picker, TITLE};
use super::sessions_fixture::{DONE, OPEN, RECORD, pane, picker};
use crate::front::tui::theme::{Charset, Glyph, glyph};

/// Whether `row` leads with `mark`, selected or not.
fn carried(row: &str, mark: &str) -> bool {
    row.starts_with(&format!("  {mark} ")) || row.starts_with(&format!("▸ {mark} "))
}

/// Every state draws its own mark, the record says it is read-only, and the
/// session this process is on carries the tag that says so.
#[test]
fn every_state_draws_its_own_mark_and_words() {
    let rows = pane(&picker(), 100, 30);

    let top = rows[0].as_str();
    assert!(top.starts_with(TITLE), "the pane names itself: {top:?}");
    assert!(
        top.contains(FILTER_HINT),
        "and says a press filters: {top:?}"
    );

    let open = rows[1].as_str();
    assert!(carried(open, OPEN), "the run in hand leads: {open:?}");
    assert!(open.contains("(this session)"), "and is tagged: {open:?}");
    assert!(
        !open.contains("read-only"),
        "and is not read-only: {open:?}"
    );

    let orders = rows[2].as_str();
    assert!(carried(orders, DONE), "a finished run: {orders:?}");
    assert!(!orders.contains("(this session)"), "{orders:?}");
    assert!(!orders.contains("read-only"), "{orders:?}");

    let revenue = rows[3].as_str();
    assert!(
        carried(revenue, RECORD),
        "a record with no run: {revenue:?}"
    );
    assert!(revenue.contains("read-only"), "says so: {revenue:?}");

    let dryrun = rows[4].as_str();
    assert!(carried(dryrun, DONE), "{dryrun:?}");

    assert_eq!(rows.len(), 30, "the pane is the height it was given");
    assert!(
        rows[29].contains("Esc close"),
        "the key row is the last row: {:?}",
        rows[29]
    );
}

/// The facts an operator chooses by are on the row: the database it was on, how
/// many turns it settled, and how long ago it was saved.
#[test]
fn a_row_carries_the_facts_it_is_chosen_by() {
    let rows = pane(&picker(), 100, 30);

    assert!(rows[1].contains("11 turns"), "{:?}", rows[1]);
    assert!(rows[1].contains("just now"), "the age of the open run");
    assert!(rows[2].contains("demo"), "{:?}", rows[2]);
    assert!(rows[2].contains("6 turns"), "{:?}", rows[2]);
    assert!(rows[2].contains("2h ago"), "{:?}", rows[2]);
    assert!(rows[3].contains("prod"), "{:?}", rows[3]);
    assert!(rows[3].contains("3 turns"), "{:?}", rows[3]);
    assert!(rows[3].contains("12d ago"), "{:?}", rows[3]);

    let selected = glyph(Glyph::Selected, Charset::Utf8);
    assert_eq!(
        rows[1..5]
            .iter()
            .filter(|row| row.contains(selected))
            .count(),
        1,
        "one row carries the selection mark: {rows:?}"
    );
    assert!(rows[1].contains(selected), "{:?}", rows[1]);
}

/// The key row names the three presses, and the confirm replaces it with the
/// question it asks.
#[test]
fn the_pane_names_its_keys_and_its_ask() {
    let mut picker = picker();
    let keys = pane(&picker, 100, 30);
    let last = keys.last().expect("the pane has a bottom border");
    assert!(last.contains("resume"), "{last:?}");
    assert!(last.contains("d delete"), "{last:?}");
    assert!(last.contains("Esc close"), "{last:?}");

    picker.ask_delete();
    let asked = pane(&picker, 100, 30);
    let last = asked.last().expect("the pane has a bottom border");
    assert!(
        last.starts_with("delete fix the failing pricing test? y/n"),
        "the confirm asks in the border's own row: {last:?}"
    );
    assert_eq!(asked.len(), 30, "and the pane keeps its height");
}

/// The pane lists the rows the picker holds, in the store's order and one row
/// each: the newest session first, the oldest last.
#[test]
fn the_pane_lists_the_rows_it_was_given() {
    let picker = picker();
    let rows = pane(&picker, 100, 30);
    let listed: Vec<&String> = rows[1..rows.len() - 1]
        .iter()
        .filter(|row| !row.is_empty())
        .collect();
    assert_eq!(
        listed.len(),
        picker.rows.len(),
        "one row per listed session: {listed:?}"
    );
    assert!(
        listed[0].contains("fix the failing pricing"),
        "the newest session leads: {:?}",
        listed[0]
    );
    assert!(
        listed[1].contains("how many orders"),
        "then the one saved an hour before: {:?}",
        listed[1]
    );
    assert!(
        listed[3].contains("add dry-run flag"),
        "and the oldest is last: {:?}",
        listed[3]
    );
}

/// The pane of `Picker::default()` says there is nothing saved, rather than
/// claiming a filter matched nothing.
#[test]
fn a_picker_with_nothing_saved_says_so() {
    let rows = pane(&Picker::default(), 100, 30);
    assert!(
        rows.iter()
            .any(|row| row.contains(super::sessions::NO_SESSIONS)),
        "a picker with nothing saved says so: {rows:?}"
    );
}
