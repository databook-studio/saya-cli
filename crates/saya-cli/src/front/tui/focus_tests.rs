//! The block cursor through the pure update: where a first `Alt+↑` lands, how
//! the walk clamps, that `Enter` reaches a group in an *older* turn, that a
//! failed group is never folded, what `c` copies, and that Esc leaves cursor
//! mode.
//!
//! The presses are the input thread's own, the conversation is the view model
//! T5 paints, and the copy is performed on T14's fake port: the one thing under
//! test is that the surface's decision is the whole decision.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::focus::EntryPath;
use super::focus_fixture::{alt_down, alt_up, answer, conversation, escape, folded_of, press};
use super::msg::Cmd;
use super::run_cmd::perform;
use super::run_fixture_port::{Call, FakePort};
use super::update::update;

#[test]
fn alt_up_from_nothing_focuses_the_newest_entry() {
    let mut model = conversation(false);
    assert_eq!(model.focus(), None, "nothing is focused to begin with");

    let cmds = update(&mut model, alt_up());

    assert_eq!(
        model.focus(),
        Some(EntryPath::new(1, 1)),
        "the first press lands on the newest entry"
    );
    assert!(
        cmds.is_empty(),
        "walking the cursor sends nothing: {cmds:?}"
    );
}

#[test]
fn the_cursor_walks_across_turns_and_stops_at_both_ends() {
    let mut model = conversation(false);
    update(&mut model, alt_up());

    let mut walked = vec![];
    for _ in 0..3 {
        update(&mut model, alt_up());
        walked.push(model.focus());
    }
    assert_eq!(
        walked,
        vec![
            Some(EntryPath::new(1, 0)),
            Some(EntryPath::new(0, 2)),
            Some(EntryPath::new(0, 1)),
        ],
        "the walk crosses the turn boundary, newest entry first"
    );

    for _ in 0..3 {
        update(&mut model, alt_up());
    }
    assert_eq!(
        model.focus(),
        Some(EntryPath::new(0, 0)),
        "up stops at the oldest entry, across every turn's boundary"
    );

    for _ in 0..9 {
        update(&mut model, alt_down());
    }
    assert_eq!(
        model.focus(),
        Some(EntryPath::new(1, 1)),
        "down stops at the newest entry"
    );
}

#[test]
fn enter_expands_a_group_in_an_older_turn() {
    let mut model = conversation(false);
    for _ in 0..4 {
        update(&mut model, alt_up());
    }
    assert_eq!(
        model.focus(),
        Some(EntryPath::new(0, 1)),
        "four presses up is the older turn's group"
    );

    let cmds = update(&mut model, press(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(
        folded_of(&model, 0),
        Some(false),
        "Enter expands a group in an older turn, not only the newest"
    );
    assert_eq!(
        folded_of(&model, 1),
        Some(true),
        "the newest turn's group is untouched"
    );
    assert!(cmds.is_empty(), "expanding a block sends no turn: {cmds:?}");
}

#[test]
fn a_failed_group_stays_expanded() {
    let mut model = conversation(true);
    for _ in 0..4 {
        update(&mut model, alt_up());
    }
    assert_eq!(
        model.focus(),
        Some(EntryPath::new(0, 1)),
        "the failed group's seat is reachable"
    );

    update(&mut model, press(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        folded_of(&model, 0),
        Some(true),
        "a failed group is never folded: the reason has to stay readable"
    );

    update(&mut model, press(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(
        folded_of(&model, 0),
        Some(true),
        "and `←` does not fold it either"
    );
}

#[test]
fn c_on_a_focused_answer_copies_its_source_and_says_so() {
    let mut model = conversation(false);
    for _ in 0..3 {
        update(&mut model, alt_up());
    }
    assert_eq!(
        model.focus(),
        Some(EntryPath::new(0, 2)),
        "the older turn's answer is the seat"
    );

    let cmds = update(&mut model, press(KeyCode::Char('c'), KeyModifiers::NONE));

    let expected = Cmd::Copy(answer());
    assert_eq!(
        cmds,
        vec![expected],
        "`c` copies the answer's source text, exactly"
    );
    let port = FakePort::new();
    for cmd in cmds {
        perform(&port, cmd);
    }
    assert_eq!(
        port.calls(),
        vec![Call::Copy(answer())],
        "the clipboard is handed the markdown source, not its painted rows"
    );
    assert_eq!(
        model.toast(),
        Some("copied answer (1.2 kB)"),
        "and `02` §4's toast says what was copied and how much"
    );
}

#[test]
fn esc_leaves_cursor_mode() {
    let mut model = conversation(false);
    update(&mut model, alt_up());
    assert!(model.focus().is_some(), "the cursor holds a seat");

    let cmds = update(&mut model, escape());

    assert_eq!(model.focus(), None, "Esc leaves the cursor where it was");
    assert!(
        cmds.is_empty(),
        "leaving the cursor sends and cancels nothing: {cmds:?}"
    );
    assert!(!model.is_busy(), "and no turn was started");
}
