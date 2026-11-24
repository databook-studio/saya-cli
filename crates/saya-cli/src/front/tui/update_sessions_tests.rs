//! The saved-session picker through the pure update: what opens it, what `↵`
//! resumes, and what a resumed session does to the transcript and the row.
//!
//! The rows come from the store in the surface itself
//! ([`super::sessions_picker`]); here they are stated as the pane's own data, so
//! these cases are about the keys and the update's decisions.

use ratatui::crossterm::event::KeyCode;

use super::model::Model;
use super::msg::{Cmd, Msg, ResumedView};
use super::update::update;
use super::update_fixture::press;
use crate::front::Entry;
use crate::front::tui::view::{SessionRow, SessionState};

/// One row, in the state a record with no attachable run is in.
fn row(id: &str, title: &str) -> SessionRow {
    SessionRow {
        id: id.to_owned(),
        title: title.to_owned(),
        database: Some("demo".to_owned()),
        turns: 2,
        age: "1 minute ago".to_owned(),
        state: SessionState::Record,
        read_only: true,
    }
}

/// The two saved sessions a case opens the pane on.
fn two() -> Msg {
    Msg::Sessions(vec![
        row("s-newer", "the newer question"),
        row("s-older", "the older question"),
    ])
}

/// What `/sessions` answers with opens the picker, and sends nothing.
#[test]
fn the_sessions_answer_opens_the_picker_on_those_rows() {
    let mut model = Model::default();

    let cmds = update(&mut model, two());

    assert!(
        cmds.is_empty(),
        "opening the pane is not a command: {cmds:?}"
    );
    let picker = model.sessions_picker().expect("the picker is open");
    assert_eq!(picker.listed().len(), 2, "both rows are listed");
    assert!(model.overlay_open(), "the pane owns the screen");
}

/// `↵` resumes the selected row: one command, and the pane closes.
#[test]
fn enter_resumes_the_selected_row_and_closes_the_pane() {
    let mut model = Model::default();
    update(&mut model, two());
    update(&mut model, press(KeyCode::Down));

    let cmds = update(&mut model, press(KeyCode::Enter));

    assert_eq!(
        cmds,
        vec![Cmd::ResumeSession("s-older".to_owned())],
        "the row under the cursor is the one resumed"
    );
    assert!(!model.overlay_open(), "the pane closes on the row it chose");
}

/// Esc closes the picker and resumes nothing.
#[test]
fn escape_closes_the_picker_without_resuming_anything() {
    let mut model = Model::default();
    update(&mut model, two());

    let cmds = update(&mut model, press(KeyCode::Esc));

    assert!(cmds.is_empty(), "Esc states no operation: {cmds:?}");
    assert!(!model.overlay_open(), "the pane closes");
    assert!(
        model.sessions_picker().is_none(),
        "and nothing is left open"
    );
}

/// Typing narrows the rows, and `↵` resumes the one the filter leaves.
///
/// The filter is typed without `d`: that character is the pane's own ask before
/// a delete ([`super::sessions_keys`]), so a case filters on a letter the two
/// titles differ in.
#[test]
fn typing_narrows_the_rows_and_enter_chooses_what_is_left() {
    let mut model = Model::default();
    update(&mut model, two());
    for character in "l".chars() {
        update(&mut model, press(KeyCode::Char(character)));
    }
    assert_eq!(
        model
            .sessions_picker()
            .expect("the picker is open")
            .listed()
            .len(),
        1,
        "the filter leaves one row"
    );

    let cmds = update(&mut model, press(KeyCode::Enter));

    assert_eq!(
        cmds,
        vec![Cmd::ResumeSession("s-older".to_owned())],
        "the filter chose the row, not the cursor"
    );
}

/// A resumed session is the conversation from there on: its last turns are the
/// transcript, under the one line the surface says, and the row names what the
/// session is on.
#[test]
fn a_resumed_session_is_the_transcript_from_there_on() {
    let mut model = Model::default();
    model.insert("a turn of the session being left");
    update(
        &mut model,
        Msg::Chat(crate::chat::ChatEvent::Agent(
            saya_agent::AgentEvent::assistant_text("its answer"),
        )),
    );

    update(
        &mut model,
        Msg::Resumed(ResumedView {
            notice: "resumed the older question — 2 turns".to_owned(),
            turns: vec![
                ("the older question".to_owned(), "its answer".to_owned()),
                ("a later one".to_owned(), "a later answer".to_owned()),
            ],
            model: "chosen-model".to_owned(),
            database: Some("demo".to_owned()),
        }),
    );

    let turns = model.turns();
    assert_eq!(turns.len(), 2, "the transcript is the record's turns");
    assert!(
        matches!(
            turns[0].entries().first(),
            Some(Entry::UserMessage(message)) if message.text == "the older question"
        ),
        "the conversation reads from its first kept turn: {:?}",
        turns[0].entries()
    );
    assert!(
        turns
            .iter()
            .flat_map(|turn| turn.entries())
            .any(|entry| matches!(
                entry,
                Entry::Notice(notice) if notice.message == "resumed the older question — 2 turns"
            )),
        "the surface says what it resumed, in the transcript"
    );
    assert_eq!(
        model.model_name(),
        Some("chosen-model"),
        "the row names the model"
    );
    assert_eq!(model.database, Some("demo".to_owned()), "and the database");
}
