//! The messages the update's tests fold: the presses a session reacts to, the
//! call put to the operator, one turn ending, and the typing helper that
//! drives the composer key by key.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Model;
use super::msg::{Cmd, Msg};
use super::update::update;
use crate::chat::{ChatEvent, ChatOutcome, Question};
use crate::front::Entry;

/// One press, as the input thread would hand it over.
fn key(code: KeyCode, modifiers: KeyModifiers) -> Msg {
    Msg::Key(KeyEvent::new(code, modifiers))
}

/// The plain Enter: send, or queue while a turn runs.
pub(super) fn enter() -> Msg {
    key(KeyCode::Enter, KeyModifiers::NONE)
}

/// The plain Esc: stop the turn, or deny the card.
pub(super) fn escape() -> Msg {
    key(KeyCode::Esc, KeyModifiers::NONE)
}

/// Shift+Tab, as crossterm reports it on the terminals we support.
pub(super) fn shift_tab() -> Msg {
    key(KeyCode::BackTab, KeyModifiers::SHIFT)
}

/// Types `text` into the composer, one key press at a time.
pub(super) fn typed(model: &mut Model, text: &str) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    for character in text.chars() {
        cmds.extend(update(
            model,
            key(KeyCode::Char(character), KeyModifiers::NONE),
        ));
    }
    cmds
}

/// The call the operator is put to, as the session asks it.
pub(super) fn card(id: u64) -> Msg {
    Msg::Chat(ChatEvent::Question(Question {
        id,
        caller: "Saya".to_owned(),
        facts: "SELECT COUNT(*) FROM orders".to_owned(),
        grant: Some("sql:demo".to_owned()),
    }))
}

/// One turn ending as the session reports it.
pub(super) fn turn_ended() -> Msg {
    Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered))
}

/// One press of a bare key, as the picker reads it.
pub(super) fn press(code: KeyCode) -> Msg {
    key(code, KeyModifiers::NONE)
}

/// A model with `/mode` typed into the composer and the picker opened by it.
pub(super) fn picker_open() -> Model {
    let mut model = Model::default();
    typed(&mut model, "/mode");
    let cmds = update(&mut model, enter());
    assert!(
        cmds.is_empty(),
        "opening the picker is not a turn: {cmds:?}"
    );
    assert!(
        model.overlay_open(),
        "/mode with no argument opens the picker"
    );
    model
}

/// The `mode:` lines the transcript holds, oldest first.
pub(super) fn mode_lines(model: &Model) -> Vec<String> {
    model
        .turns()
        .iter()
        .flat_map(|turn| turn.entries())
        .filter_map(|entry| match entry {
            Entry::Notice(notice) if notice.message.starts_with("mode: ") => {
                Some(notice.message.clone())
            }
            _ => None,
        })
        .collect()
}
