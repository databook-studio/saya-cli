//! The changes pane's keys (UI5): `Ctrl+D` opens it on the last turn that wrote,
//! the arrows and the page keys walk it, Esc closes it — and with nothing
//! changed the key is not a request at all, so it types nothing and opens
//! nothing.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::changes_fixture::{model_with_edits, read_step};
use super::keys::intent;
use super::model::Model;
use super::msg::Msg;
use super::update::update;
use crate::front::view::{Activity, Entry};

/// One press, folded through the update as the runtime folds it.
fn press(model: &mut Model, code: KeyCode, modifiers: KeyModifiers) {
    update(model, Msg::Key(KeyEvent::new(code, modifiers)));
}

/// Ctrl+D, the key that opens the changes pane.
fn ctrl_d(model: &mut Model) {
    press(model, KeyCode::Char('d'), KeyModifiers::CONTROL);
}

#[test]
fn ctrl_d_opens_the_pane_on_the_last_turn_that_wrote() {
    let mut model = model_with_edits();
    let open = intent(
        &model,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    assert!(
        open.is_some(),
        "a turn changed files, so the key is a request"
    );

    ctrl_d(&mut model);
    let changes = model.changes().expect("the changes pane is open");
    assert_eq!(changes.scroll, 0, "it opens at the first file");
}

#[test]
fn ctrl_d_with_nothing_changed_opens_nothing_and_types_nothing() {
    let mut model = Model::default();
    model.begin_turn(String::from("how many orders did we get?"));
    let turn = model.transcript.current();
    turn.push(Entry::Activity(Activity {
        steps: vec![read_step()],
        ended: true,
        ..Default::default()
    }));

    assert!(
        intent(
            &model,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
        )
        .is_none(),
        "no turn changed a file, so the key asks for nothing"
    );
    ctrl_d(&mut model);
    assert!(model.changes().is_none(), "and no pane opens");
    assert_eq!(
        model.composer(),
        "",
        "and the press types nothing into the composer"
    );
}

#[test]
fn the_pane_walks_its_rows_and_never_past_its_first() {
    let mut model = model_with_edits();
    model.open_changes();

    press(&mut model, KeyCode::Down, KeyModifiers::NONE);
    press(&mut model, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(model.changes().map(|changes| changes.scroll), Some(2));

    press(&mut model, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(model.changes().map(|changes| changes.scroll), Some(1));

    press(&mut model, KeyCode::Up, KeyModifiers::NONE);
    press(&mut model, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        model.changes().map(|changes| changes.scroll),
        Some(0),
        "the first row is the top: the pane does not scroll above it"
    );

    press(&mut model, KeyCode::PageDown, KeyModifiers::NONE);
    assert_eq!(
        model.changes().map(|changes| changes.scroll),
        Some(super::changes_press::PAGE as usize),
        "a page walks a screen of rows"
    );
    press(&mut model, KeyCode::PageUp, KeyModifiers::NONE);
    assert_eq!(model.changes().map(|changes| changes.scroll), Some(0));
}

#[test]
fn escape_closes_the_pane_and_nothing_else() {
    let mut model = model_with_edits();
    model.open_changes();
    press(&mut model, KeyCode::Esc, KeyModifiers::NONE);

    assert!(model.changes().is_none(), "Esc closes the pane");
    assert!(!model.is_busy(), "and it does not read as a turn's stop");
}

#[test]
fn the_open_pane_owns_the_keyboard() {
    let mut model = model_with_edits();
    model.open_changes();
    press(&mut model, KeyCode::Char('x'), KeyModifiers::NONE);
    press(&mut model, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        model.composer(),
        "",
        "no letter and no send reaches the composer behind the pane"
    );
    assert!(model.changes().is_some(), "the pane is still open");
}

#[test]
fn the_pane_is_shut_by_a_turn_that_lands() {
    let mut model = model_with_edits();
    model.open_changes();
    // A settled turn closes nothing by itself: the pane is the operator's, so it
    // stays until Esc. What it paints is read from the model at paint time.
    model.end_turn();
    assert!(
        model.changes().is_some(),
        "the pane outlives the turn it opened on"
    );
}
