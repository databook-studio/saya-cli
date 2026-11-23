//! `@path` mentions in the composer: the list opens on `@`, narrows as the path
//! is typed, and puts the path it points at on the line.
//!
//! The rule these cases pin is the one the approval card shares with it: while a
//! card is open and the list is closed, `Tab` is the seat toggle and never an
//! insertion, and a card arriving closes the list — the popup is the composer's,
//! and only while the composer holds the keyboard.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Model;
use super::msg::{Cmd, Msg};
use super::update::update;
use super::view::cards_fixture::shell_question;
use crate::chat::ChatEvent;

/// One character, as the input thread hands it over.
fn typed(character: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
}

/// One key that types nothing.
fn key(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Types `text` one press at a time, answering what the update asked for.
fn type_text(model: &mut Model, text: &str) -> Vec<Cmd> {
    text.chars()
        .flat_map(|character| update(model, typed(character)))
        .collect()
}

/// The paths a fixture workspace offers.
fn paths() -> Vec<String> {
    ["docs/readme.md", "src/lib.rs", "src/main.rs"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// A model with the list open on the paths a fixture walk returned.
fn offering() -> Model {
    let mut model = Model::default();
    let _ = type_text(&mut model, "@");
    assert!(update(&mut model, Msg::Mentions(paths())).is_empty());
    model
}

/// `@` opens the list and asks for the paths, once.
#[test]
fn typing_at_opens_the_list_and_asks_the_walk() {
    let mut model = Model::default();
    let cmds = type_text(&mut model, "@");
    assert_eq!(
        cmds,
        vec![Cmd::Mentions],
        "the paths are gathered off the loop's thread"
    );
    assert!(model.mention().is_some(), "the list is open");
    // A second character does not ask again: one walk serves the list.
    assert!(type_text(&mut model, "s").is_empty());
    assert!(model.mention().is_some(), "the list stays open");
}

/// What is typed after the `@` narrows the list, fuzzily, on the path.
#[test]
fn the_list_narrows_with_the_path_being_typed() {
    let mut model = offering();
    let _ = type_text(&mut model, "main");
    assert_eq!(model.mention_rows(), vec!["src/main.rs"]);
    let _ = type_text(&mut model, "zzz");
    assert!(
        model.mention_rows().is_empty(),
        "a query that matches nothing offers nothing"
    );
}

/// Tab and Enter put the selected path on the line and close the list.
#[test]
fn tab_and_enter_insert_the_selected_path() {
    for insert in [key(KeyCode::Tab), key(KeyCode::Enter)] {
        let mut model = offering();
        let _ = type_text(&mut model, "main");
        let cmds = update(&mut model, insert);
        assert_eq!(model.composer(), "@src/main.rs ", "the path is inserted");
        assert!(model.mention().is_none(), "the list closes on insertion");
        assert!(
            !cmds.iter().any(|cmd| matches!(cmd, Cmd::Send(_))),
            "inserting a path never sends the line"
        );
    }
}

/// The arrows walk the list, and the row they point at is the one inserted.
#[test]
fn the_arrows_walk_the_list_and_choose_the_row() {
    let mut model = offering();
    let _ = type_text(&mut model, "src/");
    let rows = model.mention_rows();
    assert_eq!(rows.len(), 2, "two paths under src/: {rows:?}");
    let _ = update(&mut model, key(KeyCode::Down));
    assert_eq!(model.mention().map(|list| list.selected), Some(1));
    let _ = update(&mut model, key(KeyCode::Up));
    assert_eq!(model.mention().map(|list| list.selected), Some(0));
    let _ = update(&mut model, key(KeyCode::Tab));
    assert_eq!(model.composer(), format!("@{} ", rows[0]));
}

/// Esc closes the list and keeps the draft exactly as it was typed.
#[test]
fn esc_closes_the_list_and_keeps_the_draft() {
    let mut model = offering();
    let _ = type_text(&mut model, "src");
    let cmds = update(&mut model, key(KeyCode::Esc));
    assert!(model.mention().is_none());
    assert_eq!(model.composer(), "@src", "the draft keeps what was typed");
    assert!(cmds.is_empty(), "closing the list does nothing else");
}

/// A space ends the mention: the path is named, and the list goes away.
#[test]
fn a_space_ends_the_mention() {
    let mut model = offering();
    let _ = type_text(&mut model, "src/main.rs ");
    assert!(model.mention().is_none(), "the mention ended");
    assert_eq!(model.composer(), "@src/main.rs ");
}

/// With a card open and the list closed, `Tab` is the seat toggle — never an
/// insertion, and a card arriving closes a list that was open.
#[test]
fn a_card_keeps_tab_for_the_seat_toggle() {
    let mut model = offering();
    let _ = update(&mut model, Msg::Chat(ChatEvent::Question(shell_question())));
    assert!(
        model.mention().is_none(),
        "a card closes the list it did not open"
    );
    let _ = update(&mut model, key(KeyCode::Tab));
    assert!(
        model.panel_focused(),
        "Tab gave the keyboard to the approval panel"
    );
    assert_eq!(
        model.composer(),
        "@",
        "the press inserted nothing into the draft"
    );
}
