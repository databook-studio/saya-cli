//! Shift+Tab's cycle and the card that refuses it: the four interactive modes,
//! the withheld `bypass`, and one `mode:` line per change (J6).
//!
//! `read-only` and `never` are modes of the same axis but not members of the
//! cycle (`03` §4.2: "Shift+Tab still cycles only the four interactive modes");
//! they are reachable by naming them, which is the picker's `/mode`.

use ratatui::crossterm::event::KeyCode;
use saya_agent::PermissionMode;

use super::keys::CARD_HOLDS_MODE;
use super::model::Model;
use super::msg::Cmd;
use super::update::update;
use super::update_fixture::{card, enter, mode_lines, press, shift_tab};
use crate::front::tui::view::mode_words;

#[test]
fn the_cycle_walks_ask_accept_edits_plan_and_back() {
    let mut model = Model::default();

    let first = update(&mut model, shift_tab());
    let second = update(&mut model, shift_tab());
    let third = update(&mut model, shift_tab());

    assert_eq!(
        (first, second, third),
        (
            vec![Cmd::SwitchMode(PermissionMode::AcceptEdits)],
            vec![Cmd::SwitchMode(PermissionMode::Plan)],
            vec![Cmd::SwitchMode(PermissionMode::Ask)],
        ),
        "the cycle is ask → accept-edits → plan → ask"
    );
    assert_eq!(
        mode_lines(&model),
        vec![
            String::from("mode: accept-edits"),
            String::from("mode: plan"),
            String::from("mode: ask"),
        ],
        "one line per change, in the order they happened"
    );
    assert_eq!(
        model.toast(),
        Some(mode_words::toast(PermissionMode::Ask).as_str()),
        "and the last change's toast is the one that lives"
    );
}

#[test]
fn the_cycle_skips_read_only_and_never_and_a_withheld_bypass() {
    let mut model = Model::default();
    let mut walked = Vec::new();
    for _ in 0..6 {
        let cmds = update(&mut model, shift_tab());
        assert_eq!(cmds.len(), 1, "one command per press: {cmds:?}");
        walked.push(model.mode());
    }

    assert!(
        !walked.contains(&PermissionMode::ReadOnly) && !walked.contains(&PermissionMode::Never),
        "the cycle is the four interactive modes only: {walked:?}"
    );
    assert!(
        !walked.contains(&PermissionMode::Bypass),
        "a withheld bypass is never reached: {walked:?}"
    );
}

#[test]
fn the_cycle_includes_bypass_when_the_configuration_allows_it() {
    let mut model = Model {
        allow_bypass: true,
        ..Model::default()
    };
    let mut walked = Vec::new();
    for _ in 0..4 {
        update(&mut model, shift_tab());
        walked.push(model.mode());
    }

    assert_eq!(
        walked,
        vec![
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::Bypass,
            PermissionMode::Ask,
        ],
        "with the configuration's consent the cycle includes bypass"
    );
    assert_eq!(
        mode_lines(&model),
        vec![
            String::from("mode: accept-edits"),
            String::from("mode: plan"),
            String::from("mode: bypass"),
            String::from("mode: ask"),
        ],
        "four changes, four lines"
    );
}

#[test]
fn a_card_holds_the_switch_with_one_pinned_toast() {
    let mut model = Model::default();
    update(&mut model, card(9));

    let shift_tab_cmds = update(&mut model, shift_tab());
    assert!(
        shift_tab_cmds.is_empty(),
        "no mode is stated while a card waits: {shift_tab_cmds:?}"
    );
    assert_eq!(
        model.toast(),
        Some(CARD_HOLDS_MODE),
        "the refusal is T3's one wording"
    );
    assert_eq!(model.mode(), PermissionMode::Ask, "the mode is untouched");
    assert!(!model.overlay_open(), "and no picker opens over the card");

    // `/mode` typed behind the card opens no picker: a pane never opens over a
    // card that is waiting for an answer, and what the composer holds is the
    // operator's own line — here the turn `/mode` is (K2 gives it its
    // argument). `Tab` puts the keyboard in the composer first (UI3): the card
    // answers only while it has it.
    update(&mut model, press(KeyCode::Tab));
    model.insert("/mode");
    let enter_cmds = update(&mut model, enter());
    assert_eq!(
        enter_cmds,
        vec![Cmd::Send(String::from("/mode"))],
        "the typed line is sent and the card is not answered: {enter_cmds:?}"
    );
    assert!(!model.overlay_open(), "no picker opens over the card");
    assert!(model.card().is_some(), "the card is still there to answer");
    assert!(
        mode_lines(&model).is_empty(),
        "and no change was written: {:?}",
        mode_lines(&model)
    );
}

#[test]
fn a_slash_command_the_surface_does_not_own_is_still_a_turn() {
    let mut model = Model::default();
    model.insert("/mode plan");

    let cmds = update(&mut model, enter());

    assert!(
        !model.overlay_open(),
        "only bare `/mode` opens the picker; an argument is K2's"
    );
    assert_eq!(
        cmds,
        vec![Cmd::Send(String::from("/mode plan"))],
        "what the surface does not answer itself goes to the session"
    );
    assert!(
        update(&mut model, press(KeyCode::Down)).is_empty(),
        "and no picker is there to walk"
    );
}
