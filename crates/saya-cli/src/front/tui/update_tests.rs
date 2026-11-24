//! The update's contract, driven the way the runtime drives it: one message
//! in, the commands to perform out. No terminal takes part, so a whole session
//! — sending, queueing, stopping, answering — is asserted here.
//!
//! The four rules under test are the surface's own: Enter sends and Enter
//! during a turn queues (one deep, and it runs when the turn ends), Esc stops
//! a running turn, Esc on an approval card denies that card instead of
//! stopping the turn, and Shift+Tab walks the mode cycle unless a card is open
//! — a card's answer comes first, and the key says so.

use saya_agent::PermissionMode;

use super::model::Model;
use super::msg::{Cmd, Msg, Worker};
use super::update::update;
use super::update_fixture::{card, enter, escape, shift_tab, turn_ended, typed};
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, ChatOutcome};

#[test]
fn enter_sends_the_composer_and_clears_it() {
    let mut model = Model::default();
    assert!(typed(&mut model, "count the orders").is_empty());
    assert_eq!(model.composer(), "count the orders");

    let cmds = update(&mut model, enter());

    assert_eq!(
        cmds,
        vec![Cmd::Send("count the orders".to_owned())],
        "Enter sends exactly the composer's text, as one command"
    );
    assert_eq!(model.composer(), "", "the sent text leaves the composer");
    assert!(model.is_busy(), "a sent message is a turn in flight");
    assert_eq!(
        model.turns().len(),
        1,
        "the sent message opens its own turn: {:?}",
        model.turns()
    );
}

#[test]
fn enter_while_a_turn_runs_queues_and_the_queued_text_runs_after_it() {
    let mut model = Model::default();
    typed(&mut model, "first");
    assert_eq!(
        update(&mut model, enter()),
        vec![Cmd::Send("first".to_owned())]
    );

    typed(&mut model, "second");
    let while_busy = update(&mut model, enter());

    assert!(
        while_busy.is_empty(),
        "a running turn takes the message behind it — it does not start a second turn: {while_busy:?}"
    );
    assert_eq!(
        model.composer(),
        "",
        "a queued message leaves the composer too"
    );
    assert_eq!(model.queued(), Some("second"));

    let ended = update(&mut model, turn_ended());

    assert_eq!(
        ended,
        vec![Cmd::Send("second".to_owned())],
        "the queued text is sent when the turn that held it ends"
    );
    assert_eq!(model.queued(), None, "the queue is drained by the send");
    assert!(
        model.is_busy(),
        "the queued message is a turn of its own, and it is now running"
    );
}

#[test]
fn esc_while_a_turn_runs_cancels_it() {
    let mut model = Model::default();
    typed(&mut model, "first");
    update(&mut model, enter());

    assert_eq!(
        update(&mut model, escape()),
        vec![Cmd::CancelTurn],
        "Esc over a running turn asks the runtime to cancel it"
    );
    assert!(
        model.is_busy(),
        "a cancel is a request: the turn is in flight until the session says it ended"
    );
    update(
        &mut model,
        Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Cancelled)),
    );
    assert!(
        update(&mut model, escape()).is_empty(),
        "Esc with nothing running and no card is not a request"
    );
}

#[test]
fn esc_on_a_card_denies_that_card_and_never_cancels_the_turn() {
    let mut model = Model::default();
    typed(&mut model, "how many orders?");
    update(&mut model, enter());
    update(&mut model, card(7));
    assert_eq!(model.card().map(|card| card.id), Some(7));

    let cmds = update(&mut model, escape());

    assert_eq!(
        cmds,
        vec![Cmd::Answer(7, Answer::Deny)],
        "Esc denies the call the card is showing"
    );
    assert!(
        !cmds.contains(&Cmd::CancelTurn),
        "a denied card is not a stopped turn: {cmds:?}"
    );
    assert!(model.card().is_none(), "an answered card closes");
    assert!(
        model.is_busy(),
        "the turn the card belongs to keeps running after a denial"
    );
}

#[test]
fn shift_tab_over_a_card_is_refused_with_a_toast() {
    let mut model = Model::default();
    update(&mut model, card(3));

    let cmds = update(&mut model, shift_tab());

    assert!(
        cmds.is_empty(),
        "no mode is stated while a card waits to be answered: {cmds:?}"
    );
    assert_eq!(
        model.mode(),
        PermissionMode::Ask,
        "the refused press leaves the mode exactly as it was"
    );
    let toast = model.toast().expect("the refused press is said out loud");
    assert!(
        toast.contains("answer"),
        "the toast says what has to happen first: {toast}"
    );
    assert!(
        model.card().is_some(),
        "the card is still there for the operator to answer"
    );
}

#[test]
fn shift_tab_walks_the_mode_cycle_and_states_the_mode() {
    let mut model = Model::default();

    assert_eq!(
        update(&mut model, shift_tab()),
        vec![Cmd::SwitchMode(PermissionMode::AcceptEdits)],
        "Shift+Tab asks the runtime for the next mode of the cycle"
    );
    assert_eq!(
        model.mode(),
        PermissionMode::AcceptEdits,
        "the cycle's next mode is the one in force"
    );
}

#[test]
fn shift_tab_skips_a_bypass_the_configuration_withholds() {
    let mut model = Model::default();
    update(&mut model, shift_tab());
    update(&mut model, shift_tab());
    assert_eq!(model.mode(), PermissionMode::Plan);

    assert_eq!(
        update(&mut model, shift_tab()),
        vec![Cmd::SwitchMode(PermissionMode::Ask)],
        "a cycle that may not reach bypass steps over it, back to ask"
    );
}

#[test]
fn a_pasted_block_fills_the_composer_without_sending_it() {
    let mut model = Model::default();
    let block = "select 1;\r\nfrom t";

    let cmds = update(&mut model, Msg::Paste(block.to_owned()));

    assert!(
        cmds.is_empty(),
        "a pasted block is text, never a key press to act on: {cmds:?}"
    );
    assert_eq!(model.composer(), block, "the block lands in the composer");
    assert!(
        !model.is_busy(),
        "nothing is sent by a paste, newline and all"
    );
}

#[test]
fn a_workers_word_is_said_without_touching_the_composer() {
    let mut model = Model::default();

    let cmds = update(
        &mut model,
        Msg::Worker(Worker {
            message: "the export could not be written".to_owned(),
            ..Worker::default()
        }),
    );

    assert!(
        cmds.is_empty(),
        "a worker's word asks for nothing: {cmds:?}"
    );
    assert_eq!(model.toast(), Some("the export could not be written"));
    assert_eq!(
        model.composer(),
        "",
        "a worker never types for the operator"
    );
}
