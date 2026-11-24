//! The merge point of two rules that both keep their meaning here: a bare
//! `/mode`, `/job` or `/help` opens its own pane (T18's picker, T17's drawer,
//! TX1's key sheet), and no pane ever opens over a card that is waiting for an
//! answer (the job consent's rule). With the slot free the pane opens; with a
//! card up the line is the session's to answer and the card is never covered.

use ratatui::crossterm::event::KeyCode;

use super::jobs_fixture::card;
use super::model::Model;
use super::msg::Cmd;
use super::update::update;
use super::update_fixture::press;
use crate::slash::SlashCommand;

/// The bare lines that own a pane of their own.
const PANES: [&str; 3] = ["/mode", "/job", "/help"];

/// A model with a card waiting for the operator.
fn waiting() -> Model {
    let mut model = Model::default();
    let cmds = update(&mut model, card(7));
    assert_eq!(
        cmds,
        vec![Cmd::Bell],
        "a card that takes the slot rings once"
    );
    assert_eq!(model.card().map(|card| card.id), Some(7), "the card waits");
    // The operator types behind the card, so the keyboard goes back to the
    // composer: the panel answers only when it has it (UI3).
    update(&mut model, press(KeyCode::Tab));
    model
}

/// One submitted line, as the update answered it.
fn submitted(model: &mut Model, line: &str) -> Vec<Cmd> {
    // Set whole: `/` on an empty composer opens the palette (`02` §11), so a
    // slash line reaches the composer as text.
    model.insert(line);
    update(model, press(KeyCode::Enter))
}

#[test]
fn a_pane_never_opens_over_a_card_that_waits() {
    for line in PANES {
        let mut model = waiting();
        let cmds = submitted(&mut model, line);
        assert!(
            !model.overlay_open(),
            "`{line}` opens no pane over the card: {cmds:?}"
        );
        assert!(!model.jobs_open(), "and no drawer either: `{line}`");
        assert_eq!(
            model.card().map(|card| card.id),
            Some(7),
            "the card the operator is looking at still waits: `{line}`"
        );
        assert_eq!(
            model.composer(),
            "",
            "the line is taken: it is not left half-typed behind the card"
        );
    }
}

#[test]
fn with_the_slot_free_each_bare_line_opens_its_own_pane() {
    for line in PANES {
        let mut model = Model::default();
        let cmds = submitted(&mut model, line);
        assert!(cmds.is_empty(), "`{line}` is the surface's own: {cmds:?}");
        assert!(
            model.overlay_open() || model.jobs_open(),
            "`{line}` opens its pane when no card is up"
        );
    }
}

#[test]
fn behind_a_card_the_sheet_line_is_still_the_sessions_to_answer() {
    let mut model = waiting();
    let cmds = submitted(&mut model, "/help");
    assert_eq!(
        cmds,
        vec![Cmd::Op(SlashCommand::Help(None))],
        "the pane waits for the card, so the line is answered the way a topic's is"
    );
}
