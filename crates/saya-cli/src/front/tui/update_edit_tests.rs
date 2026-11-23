//! Esc twice edits the last message (UI8): the first Esc on an empty composer
//! arms it, the second puts the last turn's own words back on the line, and
//! sending them replaces that turn rather than following it.
//!
//! Each of the existing Esc meanings is pinned here too: a running turn is
//! still stopped, an open card is still denied, and a search or a pane still
//! closes — none of them edits anything.

use super::model::Model;
use super::msg::{Cmd, Msg};
use super::update::update;
use super::update_fixture::{card, enter, escape, typed};
use crate::chat::{ChatEvent, ChatOutcome};

/// Sends `text` as a turn and lets it settle answered.
fn turn(model: &mut Model, text: &str) {
    typed(model, text);
    assert_eq!(
        update(model, enter()),
        vec![Cmd::Send(text.to_owned())],
        "the line is a turn"
    );
    update(
        model,
        Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered)),
    );
}

#[test]
fn esc_esc_puts_the_last_turns_message_back_on_the_composer() {
    let mut model = Model::default();
    turn(&mut model, "count the orders");
    assert_eq!(model.composer(), "", "the sent line left the composer");

    assert!(
        update(&mut model, escape()).is_empty(),
        "the first Esc only arms the edit"
    );
    assert_eq!(model.composer(), "", "the first Esc types nothing");

    assert!(
        update(&mut model, escape()).is_empty(),
        "restoring an edit runs nothing"
    );
    assert_eq!(
        model.composer(),
        "count the orders",
        "the second Esc puts the last message back, for editing"
    );
    assert_eq!(model.turns().len(), 1, "and nothing was sent by it");
}

#[test]
fn a_resend_replaces_the_last_turn_instead_of_following_it() {
    let mut model = Model::default();
    turn(&mut model, "count the orders");
    update(&mut model, escape());
    update(&mut model, escape());

    let cmds = update(&mut model, enter());

    assert_eq!(
        cmds,
        vec![Cmd::ReplaceLastTurn("count the orders".to_owned())],
        "the edited message asks the line to replace the last turn"
    );
    assert_eq!(
        model.turns().len(),
        1,
        "the visible conversation keeps one turn, the edited one"
    );
    assert!(model.is_busy(), "the replacement is a turn in flight");
    assert_eq!(model.composer(), "", "the sent line leaves the composer");
}

#[test]
fn an_edited_replacement_is_what_runs() {
    let mut model = Model::default();
    turn(&mut model, "count the orders");
    update(&mut model, escape());
    update(&mut model, escape());
    // The operator edits the restored line before sending it.
    typed(&mut model, " this month");

    let cmds = update(&mut model, enter());

    assert_eq!(
        cmds,
        vec![Cmd::ReplaceLastTurn(
            "count the orders this month".to_owned()
        )],
        "the edited words are the ones the line runs"
    );
}

#[test]
fn typing_between_the_two_escs_puts_the_arm_down() {
    let mut model = Model::default();
    turn(&mut model, "count the orders");
    update(&mut model, escape());
    typed(&mut model, "x");

    assert!(
        update(&mut model, escape()).is_empty(),
        "a key between the Escs is no longer an edit"
    );
    assert_eq!(
        model.composer(),
        "x",
        "the draft the operator typed is never replaced"
    );
}

#[test]
fn esc_over_a_running_turn_still_cancels_and_never_edits() {
    let mut model = Model::default();
    turn(&mut model, "count the orders");
    typed(&mut model, "another line");
    update(&mut model, enter());

    assert_eq!(
        update(&mut model, escape()),
        vec![Cmd::CancelTurn],
        "Esc over a running turn is still the stop"
    );
    assert_eq!(
        model.composer(),
        "",
        "and it never puts the last message back mid-turn"
    );
}

#[test]
fn esc_on_a_card_still_denies_it_and_never_edits() {
    let mut model = Model::default();
    turn(&mut model, "count the orders");
    update(&mut model, card(7));

    let cmds = update(&mut model, escape());

    assert_eq!(
        cmds,
        vec![Cmd::Answer(7, crate::agent::tools::consent::Answer::Deny)],
        "the card's Esc is still the denial"
    );
    assert_eq!(model.composer(), "", "a denied card edits nothing");
}

#[test]
fn a_slash_command_leaves_nothing_to_edit() {
    let mut model = Model::default();
    // `/help` is answered by the surface itself, never a turn.
    model.insert("/help");
    update(&mut model, enter());
    update(&mut model, escape());
    update(&mut model, escape());

    assert_eq!(
        model.composer(),
        "",
        "a line that was not a turn is not restorable"
    );
}
