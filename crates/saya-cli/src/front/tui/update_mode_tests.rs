//! `/mode`'s picker through the pure update: what opens it, what a row set
//! states, and what a cancel says — nothing.
//!
//! The words read here are the pane's own (`view::mode_words`), so a change
//! that said one thing in the status line and another in the transcript would
//! fail.

use ratatui::crossterm::event::KeyCode;
use saya_agent::PermissionMode;

use super::model::Model;
use super::msg::Cmd;
use super::update::update;
use super::update_fixture::{enter, escape, mode_lines, picker_open, press, typed};
use crate::front::tui::view::mode_words;

#[test]
fn slash_mode_with_no_argument_opens_the_picker_and_sends_nothing() {
    let mut model = Model::default();
    typed(&mut model, "/mode");

    let cmds = update(&mut model, enter());

    assert!(
        cmds.is_empty(),
        "`/mode` is the surface's, not a turn: {cmds:?}"
    );
    assert!(model.overlay_open(), "the picker is open");
    assert_eq!(
        model.composer(),
        "",
        "the typed command leaves the composer"
    );
    assert!(!model.is_busy(), "nothing was sent");
    assert!(
        mode_lines(&model).is_empty(),
        "opening the picker is not a change: {:?}",
        mode_lines(&model)
    );
    assert_eq!(
        model.mode_picker().map(|picker| picker.chosen()),
        Some(PermissionMode::Ask),
        "the picker opens on the mode in force"
    );
}

#[test]
fn the_picker_walks_its_rows_and_stops_at_both_ends() {
    let mut model = picker_open();

    update(&mut model, press(KeyCode::Up));
    assert_eq!(
        model.mode_picker().map(|picker| picker.chosen()),
        Some(PermissionMode::Ask),
        "up on the first row stays on it"
    );
    for _ in 0..9 {
        update(&mut model, press(KeyCode::Down));
    }
    assert_eq!(
        model.mode_picker().map(|picker| picker.chosen()),
        Some(PermissionMode::Never),
        "down stops at the last row"
    );
}

#[test]
fn enter_on_read_only_states_one_set_mode() {
    let mut model = picker_open();
    for _ in 0..4 {
        update(&mut model, press(KeyCode::Down));
    }
    assert_eq!(
        model.mode_picker().map(|picker| picker.chosen()),
        Some(PermissionMode::ReadOnly),
        "four rows down from ask is read-only"
    );

    let cmds = update(&mut model, enter());

    assert_eq!(
        cmds,
        vec![Cmd::SetMode(PermissionMode::ReadOnly)],
        "Enter states the selected mode, once"
    );
    assert_eq!(model.mode(), PermissionMode::ReadOnly, "and it is in force");
    assert!(!model.overlay_open(), "the picker closes on Enter");
    assert_eq!(
        mode_lines(&model),
        vec![String::from("mode: read-only")],
        "one compact line at the point of change"
    );
    assert_eq!(
        model.toast(),
        Some(mode_words::toast(PermissionMode::ReadOnly).as_str()),
        "and the toast says what the mode means"
    );
}

#[test]
fn esc_on_the_picker_asks_for_nothing() {
    let mut model = picker_open();
    update(&mut model, press(KeyCode::Down));

    let cmds = update(&mut model, escape());

    assert!(cmds.is_empty(), "Esc cancels: it states no mode: {cmds:?}");
    assert!(!model.overlay_open(), "and it closes the picker");
    assert_eq!(model.mode(), PermissionMode::Ask, "the mode is untouched");
    assert!(
        mode_lines(&model).is_empty(),
        "a cancel writes no line: {:?}",
        mode_lines(&model)
    );
}

#[test]
fn choosing_the_mode_already_in_force_writes_nothing() {
    let mut model = picker_open();

    let cmds = update(&mut model, enter());

    assert!(
        cmds.is_empty(),
        "nothing changed, so nothing is stated: {cmds:?}"
    );
    assert!(!model.overlay_open(), "the picker still closes");
    assert!(
        mode_lines(&model).is_empty(),
        "and no line claims a change: {:?}",
        mode_lines(&model)
    );
}

#[test]
fn the_picker_takes_the_keyboard_from_the_composer() {
    let mut model = picker_open();

    let cmds = update(&mut model, press(KeyCode::Char('x')));

    assert!(
        !model.composer().contains('x'),
        "no press reaches the composer behind the picker: {:?}",
        model.composer()
    );
    assert!(cmds.is_empty(), "and nothing is sent: {cmds:?}");
    assert!(model.overlay_open(), "the picker is still up");
}
