//! The picker's decisions, read without a terminal: what a resume decision
//! makes a row, what the filter keeps, where the selection stops, and the two
//! presses that could delete something.

use super::sessions::{Command, Picker};
use super::sessions_fixture::{NOW, attach, finished, listed, picker, read_only, refused};
use super::sessions_row::State;

/// Every decision the chat layer makes has one state on the row, and a record
/// with no run is the only row that opens read-only.
#[test]
fn the_link_decisions_map_to_the_three_states() {
    let open = listed(
        "a",
        "fix the failing pricing test",
        None,
        11,
        NOW - 30_000,
        attach(),
        NOW,
    );
    assert_eq!(open.state, State::Open, "a run this process holds is open");
    assert!(!open.read_only, "an open run is not read-only");
    assert_eq!(open.age, "just now", "the age is the store's stamp");

    let done = listed(
        "b",
        "how many orders did we get since December?",
        Some("demo"),
        6,
        NOW - 7_200_000,
        finished(),
        NOW,
    );
    assert_eq!(done.state, State::Finished, "a finished run is finished");
    assert!(
        !done.read_only,
        "a finished run resumes into a fresh run, not read-only"
    );
    assert_eq!(done.age, "2h ago");

    let record = listed(
        "c",
        "revenue by region (0.5.0)",
        Some("prod"),
        3,
        NOW - 1_036_800_000,
        read_only(),
        NOW,
    );
    assert_eq!(
        record.state,
        State::Record,
        "a record with no run is a record"
    );
    assert!(record.read_only, "a record with no run opens read-only");
    assert_eq!(record.age, "12d ago");

    let refused = listed(
        "d",
        "a job in a managed checkout",
        None,
        2,
        NOW,
        refused(),
        NOW,
    );
    assert_eq!(
        refused.state,
        State::Record,
        "a refusal has no run to open either"
    );
    assert!(
        refused.read_only,
        "so it opens read-only rather than offering a resume that is refused"
    );

    assert_eq!(record.read_only, State::Record.read_only());
    assert!(!State::Open.read_only() && !State::Finished.read_only());
}

/// The filter narrows by the title and by the database, and a filter that
/// matches nothing leaves nothing to act on.
#[test]
fn the_filter_narrows_by_title_and_by_database() {
    let mut picker = picker();
    assert_eq!(picker.listed().len(), 4, "the picker lists every record");

    for character in "pricing".chars() {
        picker.type_char(character);
    }
    assert_eq!(picker.listed().len(), 1, "only the title carries pricing");
    assert_eq!(picker.listed()[0].id, "sess-open");
    assert_eq!(picker.selected(), 0, "the filter starts at the first row");

    for _ in 0.."pricing".len() {
        picker.backspace();
    }
    assert_eq!(picker.listed().len(), 4, "the filter is what narrows it");

    for character in "PROD".chars() {
        picker.type_char(character);
    }
    assert_eq!(picker.listed().len(), 1, "the database narrows it too");
    assert_eq!(picker.listed()[0].id, "sess-revenue");
    assert_eq!(
        picker.listed()[0].database.as_deref(),
        Some("prod"),
        "typing a database name is case-blind"
    );

    for character in "zzzz".chars() {
        picker.type_char(character);
    }
    assert!(picker.listed().is_empty(), "nothing matches PRODzzzz");
    assert!(
        picker.selected_row().is_none(),
        "a filter that leaves nothing selects nothing"
    );
    assert_eq!(picker.resume(), None, "and there is no row to resume");
}

/// `d` asks first: it deletes nothing, and only a `y` answering the confirm
/// yields the delete command.
#[test]
fn d_asks_before_it_deletes() {
    let mut picker = picker();
    let id = picker.selected_row().expect("a row is selected").id.clone();
    let title = picker
        .selected_row()
        .expect("a row is selected")
        .title
        .clone();

    assert_eq!(
        picker.ask_delete(),
        None,
        "d asks; it never deletes on its own"
    );
    assert!(picker.is_confirming(), "the confirm is now asking");
    assert!(picker.is_open(), "and the picker is still up");
    assert_eq!(
        picker.ask().as_deref(),
        Some(format!("delete {title}? y/n").as_str()),
        "the confirm names the row it would delete"
    );
    assert_eq!(
        picker.listed().len(),
        4,
        "nothing has been taken off the list"
    );

    assert_eq!(
        picker.answer(false),
        None,
        "n withdraws the ask without a command"
    );
    assert!(!picker.is_confirming(), "the confirm is closed again");
    assert!(picker.is_open(), "n leaves the picker up");
    assert_eq!(picker.ask(), None, "and nothing is being asked");

    picker.ask_delete();
    assert_eq!(
        picker.answer(true),
        Some(Command::Delete { id }),
        "only y yields the delete, for the row it named"
    );
    assert!(
        !picker.is_confirming(),
        "the confirm is closed by the answer"
    );
}

/// Esc closes the picker and yields no command; on the confirm it withdraws the
/// ask and leaves the picker up.
#[test]
fn esc_closes_without_a_command() {
    let mut closed = picker();
    assert_eq!(closed.escape(), None, "Esc asks the runtime for nothing");
    assert!(!closed.is_open(), "Esc closes the picker");
    assert!(
        closed.selected_row().is_none(),
        "a closed picker selects nothing"
    );
    assert_eq!(closed.resume(), None, "so there is nothing to resume");
    assert_eq!(closed.ask_delete(), None, "and nothing to delete");

    let mut asked = picker();
    asked.ask_delete();
    assert!(asked.is_confirming());
    assert_eq!(asked.escape(), None, "Esc withdraws the ask, silently");
    assert!(!asked.is_confirming(), "the confirm is closed");
    assert!(asked.is_open(), "the picker is not: only the ask was");
    assert_eq!(asked.listed().len(), 4, "nothing was deleted");
}

/// The selection moves by one and stops at both ends of what the filter leaves.
#[test]
fn the_selection_stops_at_both_ends() {
    let mut picker = picker();
    for _ in 0..10 {
        picker.down();
    }
    assert_eq!(picker.selected(), 3, "the last row is the last row");
    for _ in 0..10 {
        picker.up();
    }
    assert_eq!(picker.selected(), 0, "the first row is the first row");

    for character in "revenue".chars() {
        picker.type_char(character);
    }
    assert_eq!(picker.selected(), 0, "the filter starts again at the top");

    let mut empty = Picker::default();
    empty.down();
    empty.up();
    assert_eq!(
        empty.selected_row(),
        None,
        "an empty picker selects nothing"
    );
    assert_eq!(empty.resume(), None);
    assert_eq!(empty.ask_delete(), None);
}
