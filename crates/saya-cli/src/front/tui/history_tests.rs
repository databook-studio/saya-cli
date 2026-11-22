//! The composer's input history (`02` §7): `Up`/`Down` walking the ring today's
//! surface keeps, the draft restored past its newest entry, and the `Ctrl+R`
//! search over it — including what it draws on the composer's line.
//!
//! The ring itself (its bounds, its de-duplication, its `--param` redaction) is
//! `interactive::tui::history`'s and is pinned there; the two copies a submitted
//! line leaves behind are `history_echo_tests`'s. These tests pin what the new
//! composer does with the ring.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Model;
use super::msg::Msg;
use super::update::update;
use super::update_fixture::{enter, escape, press, typed};
use super::view::Row;

/// One press, as the input thread would hand it over.
fn key(code: KeyCode, modifiers: KeyModifiers) -> Msg {
    Msg::Key(KeyEvent::new(code, modifiers))
}

fn up() -> Msg {
    press(KeyCode::Up)
}

fn down() -> Msg {
    press(KeyCode::Down)
}

fn ctrl_r() -> Msg {
    key(KeyCode::Char('r'), KeyModifiers::CONTROL)
}

#[test]
fn up_walks_the_ring_and_down_restores_the_draft_past_the_newest_entry() {
    let mut model = Model::default();
    model.remember("explain select 1");
    model.remember("select count(*) from events");
    model.insert("dra");

    update(&mut model, up());
    assert_eq!(
        model.composer(),
        "select count(*) from events",
        "Up puts the newest entry on the line"
    );
    update(&mut model, up());
    assert_eq!(
        model.composer(),
        "explain select 1",
        "a second Up steps one entry older"
    );
    update(&mut model, down());
    assert_eq!(
        model.composer(),
        "select count(*) from events",
        "Down steps back towards the live edge"
    );
    update(&mut model, down());
    assert_eq!(
        model.composer(),
        "dra",
        "Down past the newest entry restores the draft that was there"
    );
}

#[test]
fn an_empty_ring_leaves_the_draft_alone() {
    let mut model = Model::default();
    model.insert("dra");

    assert!(
        update(&mut model, up()).is_empty(),
        "a recall sends nothing"
    );

    assert_eq!(
        model.composer(),
        "dra",
        "with nothing to recall the composer keeps its draft"
    );
}

#[test]
fn ctrl_r_finds_an_entry_and_enter_takes_it() {
    let mut model = Model::default();
    for line in [
        "SELECT * FROM orders",
        "explain select 1",
        "select count(*) from events",
    ] {
        model.remember(line);
    }
    model.insert("draft");

    update(&mut model, ctrl_r());
    assert_eq!(
        model.search().map(|search| search.query().to_owned()),
        Some(String::new()),
        "Ctrl+R opens the search on an empty query"
    );
    typed(&mut model, "select");

    assert_eq!(
        model.search_found(),
        Some("select count(*) from events".to_owned()),
        "the search finds the newest entry holding the query"
    );
    update(&mut model, enter());

    assert_eq!(
        model.composer(),
        "select count(*) from events",
        "Enter puts the found entry on the line"
    );
    assert!(model.search().is_none(), "and closes the search");
}

#[test]
fn a_second_ctrl_r_steps_older_and_esc_leaves_the_draft_alone() {
    let mut model = Model::default();
    model.remember("select a from t");
    model.remember("select b from t");
    model.insert("dra");

    update(&mut model, ctrl_r());
    typed(&mut model, "select");
    assert_eq!(
        model.search_found(),
        Some("select b from t".to_owned()),
        "the search opens on its newest match"
    );
    update(&mut model, ctrl_r());
    assert_eq!(
        model.search_found(),
        Some("select a from t".to_owned()),
        "a second Ctrl+R steps one match older"
    );
    update(&mut model, ctrl_r());
    assert_eq!(
        model.search_found(),
        Some("select a from t".to_owned()),
        "the oldest match is where stepping stops"
    );

    update(&mut model, escape());

    assert!(model.search().is_none(), "Esc closes the search");
    assert_eq!(
        model.composer(),
        "dra",
        "a cancelled search leaves the draft exactly as it was"
    );
}

#[test]
fn a_card_or_a_pane_keeps_the_history_keys_to_itself() {
    let mut model = Model::default();
    model.remember("older line");
    model.insert("draft");
    update(&mut model, super::update_fixture::card(1));

    update(&mut model, up());
    update(&mut model, ctrl_r());

    assert_eq!(
        model.composer(),
        "draft",
        "a card owns the keyboard, so no recall reaches the composer"
    );
    assert!(
        model.search().is_none(),
        "and Ctrl+R opens no search behind a card"
    );

    let mut pane = Model::default();
    pane.remember("older line");
    update(&mut pane, super::update_fixture::ctrl_k());
    update(&mut pane, up());
    update(&mut pane, ctrl_r());

    assert_eq!(
        pane.composer(),
        "",
        "the palette owns the keyboard, so no recall reaches the composer"
    );
    assert!(
        pane.search().is_none(),
        "and Ctrl+R narrows the palette instead of opening a search"
    );
}

#[test]
fn the_open_search_is_painted_with_its_query_and_its_match() {
    let mut model = Model::default();
    model.remember("select count(*) from events");
    update(&mut model, ctrl_r());
    typed(&mut model, "count");

    let rows: Vec<String> = super::view::rows(&model, 72, super::theme::Charset::Utf8)
        .iter()
        .map(Row::text)
        .collect();

    assert!(
        rows.iter()
            .any(|row| row.contains("(reverse-i-search)`count`: select count(*) from events")),
        "the search shows what is being looked for and what it found: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("Enter accept · Ctrl+R older · Esc cancel")),
        "the hint row names the keys a search reads: {rows:?}"
    );
}
