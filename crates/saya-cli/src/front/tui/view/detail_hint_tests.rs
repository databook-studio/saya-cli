//! The one key that opens the transcript's steps — `Ctrl+O`: named in the
//! composer's hint row, listed in the key sheet, read by the keymap from the
//! composer and from the block cursor, and carried out by the update.
//!
//! A key an operator cannot find is not a key: the whole unit rests on the
//! surface saying the chord where the operator already looks.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::composer::{Row, State, hint, state};
use super::help::{self, Context};
use crate::front::tui::focus::EntryPath;
use crate::front::tui::keys::{Intent, intent};
use crate::front::tui::model::Model;
use crate::front::tui::msg::Msg;
use crate::front::tui::update::update;

/// `Ctrl+O`, as the input thread reports it.
fn ctrl_o() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL))
}

#[test]
fn the_composer_hint_names_the_detail_key() {
    for width in [100, 80, 60] {
        let idle = Model::default();
        assert!(
            hint(&idle, width).contains("Ctrl+O"),
            "{width}: the idle hint row names the key that opens the steps: {:?}",
            hint(&idle, width)
        );
        let busy = Model {
            busy: true,
            ..Model::default()
        };
        assert!(
            hint(&busy, width).contains("Ctrl+O"),
            "{width}: and so does the running turn's, where the steps arrive: {:?}",
            hint(&busy, width)
        );
        assert_eq!(
            state(&busy),
            State::TurnRunning,
            "the running row is the one being read"
        );
    }
}

#[test]
fn the_key_sheet_lists_the_detail_key() {
    for (width, height) in [(100, 30), (60, 24)] {
        let sheet = help::rows(Context::default(), width, height)
            .iter()
            .map(Row::text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            sheet.contains("Ctrl+O") && sheet.contains("full"),
            "{width}x{height}: the sheet says the key and what it does: {sheet}"
        );
    }
}

#[test]
fn the_keymap_reads_ctrl_o_as_the_detail_toggle() {
    let chord = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(
        intent(&Model::default(), chord),
        Some(Intent::ToggleDetail),
        "the composer's Ctrl+O opens the transcript's detail"
    );

    let focused = Model {
        focus: Some(EntryPath::new(0, 0)),
        ..Model::default()
    };
    assert_eq!(
        intent(&focused, chord),
        Some(Intent::ToggleDetail),
        "the cursor's Ctrl+O does the same: the key is the surface's, not a block's"
    );

    // A bare `o` is still the character it always was.
    assert_eq!(
        intent(
            &Model::default(),
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)
        ),
        Some(Intent::Insert('o')),
        "typing an `o` is typing an `o`"
    );
}

#[test]
fn ctrl_o_turns_detail_on_and_back_off() {
    let mut model = Model::default();
    assert!(!model.detail(), "the compact transcript is the default");

    let cmds = update(&mut model, ctrl_o());
    assert!(cmds.is_empty(), "the toggle asks the runtime for nothing");
    assert!(model.detail(), "one press opens every step");

    update(&mut model, ctrl_o());
    assert!(!model.detail(), "and one press folds it back");
    assert_eq!(model.composer(), "", "the key never types itself");
}

#[test]
fn a_tick_turns_the_spinner_and_the_toast_clock_with_it() {
    let mut model = Model::default();
    assert_eq!(model.spinner(), 0);

    update(&mut model, Msg::Tick);
    update(&mut model, Msg::Tick);

    assert_eq!(
        model.spinner(),
        2,
        "every tick is one frame of a running step's spinner"
    );
}
