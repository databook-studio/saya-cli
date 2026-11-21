//! The help sheet (`02` §11), read as the keys it lists and the words it paints.
//!
//! The sheet is contextual: every row is valid in a state, and the state the
//! surface is in decides which of them an operator sees. The last test is the
//! guard that keeps the sheet and the keymap one keyboard — every press the
//! sheet lists is pressed through `keys.rs`, so a row nothing handles fails.

use ratatui::crossterm::event::KeyCode;

use super::help::{self, Context};
use super::help_fixture::{
    busy, chords, ctrl_g, idle, model_in, plain, probes, queued, row, with_card,
    with_ungranted_card, words,
};
use super::help_keys::{SHORTCUTS, When};
use crate::front::tui::cards_keys::card_intent;
use crate::front::tui::keys::{Intent, intent};
use crate::front::tui::model::Model;
use crate::front::tui::view::cards_fixture::shell_question;
use crate::interactive::tui::wrap::cell_width;

#[test]
fn a_sheet_without_a_card_lists_no_approval_key() {
    let question = shell_question();
    let context = idle();

    assert!(
        probes()
            .into_iter()
            .any(|probe| card_intent(&question, probe.event()).is_some()),
        "the card does answer presses, so the sheet has something to withhold"
    );
    for probe in probes() {
        if card_intent(&question, probe.event()).is_some() {
            assert!(
                !chords(&context).contains(&probe),
                "`{:?}` answers a card, and no card is open: {:?}",
                probe.code,
                chords(&context)
            );
        }
    }

    let sheet = words(context, 100, 30);
    assert!(
        !sheet.iter().any(|line| line.contains("allow")),
        "nothing to allow without a card: {sheet:?}"
    );
    assert!(
        !sheet.iter().any(|line| line.contains("deny")),
        "nothing to deny without a card: {sheet:?}"
    );
    assert!(
        !sheet.iter().any(|line| line.contains("card")),
        "the sheet does not speak of a card it is not showing: {sheet:?}"
    );
}

#[test]
fn a_sheet_with_a_card_lists_exactly_the_card_keys() {
    let context = with_card();
    let listed = chords(&context);
    let question = shell_question();

    for code in [
        KeyCode::Char('y'),
        KeyCode::Char('a'),
        KeyCode::Char('s'),
        KeyCode::Char('n'),
        KeyCode::Char('d'),
        KeyCode::Esc,
        KeyCode::Char('v'),
    ] {
        assert!(
            listed.contains(&plain(code)),
            "`{code:?}` is one of `02` §5's card keys: {listed:?}"
        );
    }

    for probe in &listed {
        assert!(
            card_intent(&question, probe.event()).is_some(),
            "the sheet lists `{:?}`, which the open card does not answer",
            probe.code
        );
    }
}

#[test]
fn a_card_that_offers_no_token_lists_no_session_grant() {
    let grant = plain(KeyCode::Char('s'));

    assert!(
        chords(&with_card()).contains(&grant),
        "a card that offered a token is answered with a session grant"
    );
    assert!(
        !chords(&with_ungranted_card()).contains(&grant),
        "a card that offered no token has none to grant: {:?}",
        chords(&with_ungranted_card())
    );
}

#[test]
fn the_queue_key_appears_only_while_a_line_is_queued() {
    assert!(
        chords(&queued()).contains(&ctrl_g()),
        "Ctrl+G drops the held line, so it is listed while one is held"
    );
    assert!(
        !chords(&idle()).contains(&ctrl_g()),
        "with nothing held there is nothing to drop"
    );
    assert!(
        !chords(&busy()).contains(&ctrl_g()),
        "a running turn with an empty queue has nothing to drop"
    );
}

#[test]
fn the_turn_stop_key_appears_only_while_busy() {
    let escape = plain(KeyCode::Esc);

    let stop = row(&busy(), escape).expect("a running turn is stopped with Esc");
    assert!(
        stop.action.contains("stop"),
        "the busy row says what Esc stops: {}",
        stop.action
    );
    assert!(
        row(&idle(), escape).is_none(),
        "with nothing running Esc asks for nothing"
    );

    let deny = row(&with_card(), escape).expect("an open card is denied with Esc");
    assert!(
        deny.action.contains("deny"),
        "over a card Esc denies it, and the sheet says so: {}",
        deny.action
    );
    assert_ne!(
        stop.action, deny.action,
        "one key, two states, two different words"
    );
}

#[test]
fn the_sheet_files_its_rows_under_three_columns() {
    let idle_sheet = words(idle(), 100, 30);
    assert!(
        idle_sheet.iter().any(|line| line == "Conversation"),
        "the idle sheet has a Conversation column: {idle_sheet:?}"
    );
    assert!(
        idle_sheet.iter().any(|line| line == "Navigation"),
        "the idle sheet has a Navigation column: {idle_sheet:?}"
    );

    let card_sheet = words(with_card(), 100, 30);
    assert!(
        card_sheet.iter().any(|line| line == "Copy & view"),
        "a card's withheld body is looked at, so the column is there: {card_sheet:?}"
    );
    assert!(
        card_sheet.iter().any(|line| line.contains("Help")),
        "the sheet names itself: {card_sheet:?}"
    );
}

#[test]
fn no_row_is_wider_than_the_sheet() {
    let contexts = [
        ("idle", idle()),
        ("busy", busy()),
        ("queued", queued()),
        ("card", with_card()),
        ("ungranted card", with_ungranted_card()),
    ];
    for (name, context) in contexts {
        for (width, height) in [(100, 30), (80, 24), (60, 12), (24, 8), (24, 2), (24, 1)] {
            let rows = help::rows(context, width, height);
            assert_eq!(
                rows.len(),
                height,
                "{name} at {width}x{height} is a full sheet"
            );
            for row in &rows {
                assert!(
                    cell_width(&row.text()) <= width.max(12),
                    "{:?} is wider than {width} at {name}: {}",
                    row.text(),
                    cell_width(&row.text())
                );
            }
        }
    }
}

#[test]
fn every_key_the_sheet_shows_is_handled_by_the_keymap() {
    let mut pressed = 0;
    for shortcut in SHORTCUTS {
        assert!(
            !shortcut.chords.is_empty(),
            "`{}` names no press, so nothing can check it",
            shortcut.label
        );
        let model = model_in(shortcut.when);
        for chord in shortcut.chords {
            match intent(&model, chord.event()) {
                Some(Intent::Refused(reason)) => panic!(
                    "the sheet lists `{}` ({:?}), and the keymap refuses it in its own state: {reason}",
                    shortcut.label, chord.code
                ),
                Some(_) => pressed += 1,
                None => panic!(
                    "the sheet lists `{}` ({:?}), and the keymap handles no such press in {:?}",
                    shortcut.label, chord.code, shortcut.when
                ),
            }
        }
    }
    assert!(
        pressed >= SHORTCUTS.len(),
        "every shortcut was pressed through the keymap: {pressed} presses"
    );
    assert!(
        SHORTCUTS.iter().any(|shortcut| shortcut.when == When::Card),
        "the table carries the card's own keys"
    );
    assert_eq!(
        Context::of(&Model::default()),
        idle(),
        "an untouched model is the idle context"
    );
}
