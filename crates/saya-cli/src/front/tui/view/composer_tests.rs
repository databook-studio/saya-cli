//! The composer's contract, row by row against `02` §7's state table, plus
//! the two rules the table states around it: the queue is one deep and Ctrl+G
//! drops it, and a bracketed paste is one event that lands intact.
//!
//! Every assertion is the table's own text, so a change to the copy or to the
//! state a press resolves to fails here rather than in a snapshot nobody read.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::composer::{Row, State, hint, placeholder, state, title};
use super::composer_frame::rows;
use crate::front::tui::model::Model;
use crate::front::tui::msg::Msg;
use crate::front::tui::theme::{Charset, Token};
use crate::front::tui::update::update;
use crate::front::tui::update_fixture::{enter, typed};

/// The box as text, one row per line: what a pane would draw.
fn render(model: &Model) -> String {
    text(&rows(model, 80, Charset::Utf8))
}

/// The rows as text, one row per line.
fn text(rows: &[Row]) -> String {
    rows.iter().map(Row::text).collect::<Vec<_>>().join("\n")
}

/// Ctrl+G, as the input thread reports it.
fn ctrl_g() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
}

/// A turn running with one line held behind it.
fn queued_model() -> Model {
    let mut model = Model::default();
    typed(&mut model, "first");
    update(&mut model, enter());
    typed(&mut model, "second");
    update(&mut model, enter());
    model
}

#[test]
fn idle_with_a_database_states_the_table_row() {
    let model = Model {
        database: Some("demo".to_owned()),
        ..Default::default()
    };

    assert_eq!(state(&model), State::IdleWithDatabase);
    assert_eq!(title(state(&model)), "Message", "`02` §7: idle, db");
    assert_eq!(
        placeholder(&model),
        "Ask about demo or describe a change",
        "`02` §7: the placeholder names the connected database"
    );
    assert_eq!(
        hint(&model, 100),
        "Enter send · Alt+Enter newline · Ctrl+O detail · Ctrl+K commands",
        "`02` §7: idle, db, full layout"
    );
    assert_eq!(
        hint(&model, 80),
        "Enter send · Ctrl+O detail · Ctrl+K commands",
        "80x24 keeps the hint row to the keys an operator needs"
    );
    let narrow = render(&model);
    let wide = text(&rows(&model, 100, Charset::Utf8));
    assert!(
        narrow.contains("Ask about demo or describe a change"),
        "{narrow}"
    );
    assert!(
        wide.contains("Enter send · Alt+Enter newline · Ctrl+O detail · Ctrl+K commands"),
        "{wide}"
    );
    assert!(
        narrow.contains("Enter send · Ctrl+O detail · Ctrl+K commands"),
        "{narrow}"
    );
}

#[test]
fn idle_without_a_database_states_the_table_row() {
    let model = Model::default();

    assert_eq!(state(&model), State::IdleWithoutDatabase);
    assert_eq!(title(state(&model)), "Message", "`02` §7: idle, no db");
    assert_eq!(
        placeholder(&model),
        "Describe a change or ask about this code"
    );
    assert_eq!(
        hint(&model, 100),
        "Enter send · Alt+Enter newline · Ctrl+O detail · Ctrl+K commands",
        "`02` §7 gives the idle, no db row the same hint"
    );
    assert_eq!(
        hint(&model, 80),
        "Enter send · Ctrl+O detail · Ctrl+K commands"
    );
}

#[test]
fn a_running_turn_states_the_table_row() {
    let mut model = Model::default();
    typed(&mut model, "first");
    update(&mut model, enter());

    assert_eq!(state(&model), State::TurnRunning);
    assert_eq!(title(state(&model)), "Message (queued on Enter)");
    assert_eq!(
        placeholder(&model),
        "Type a follow-up — it is sent when this turn ends"
    );
    assert_eq!(
        hint(&model, 100),
        "Enter queue · Ctrl+O detail · Esc stop turn"
    );
    assert_eq!(
        hint(&model, 80),
        "Enter queue · Ctrl+O detail · Esc stop turn",
        "the running hint names the steps' key and is short enough for 80 columns"
    );
}

#[test]
fn one_queued_line_states_the_table_row_and_is_drawn_dim() {
    let model = queued_model();

    assert_eq!(state(&model), State::OneQueued);
    assert_eq!(title(state(&model)), "Queued");
    assert_eq!(placeholder(&model), "second", "the queued text is the body");
    assert_eq!(hint(&model, 80), "↑ edit queued · Ctrl+G drop");

    let rendered = rows(&model, 80, Charset::Utf8);
    let body = rendered.get(1).expect("the queued line is a body row");
    assert!(
        body.text().contains("second"),
        "the held line is shown: {}",
        body.text()
    );
    assert_eq!(
        body.runs[0].token,
        Token::Dim,
        "`02` §7 shows the queued text dim"
    );
    let all = render(&model);
    assert!(all.contains("Queued"), "the title is drawn: {all}");
    assert!(all.contains("↑ edit queued · Ctrl+G drop"), "{all}");
}

#[test]
fn a_paused_run_states_the_table_row() {
    let model = Model {
        database: Some("demo".to_owned()),
        paused: true,
        ..Default::default()
    };

    assert_eq!(state(&model), State::Paused);
    assert_eq!(title(state(&model)), "Message");
    assert_eq!(
        placeholder(&model),
        "Enter to continue, or type to redirect"
    );
    assert_eq!(hint(&model, 100), "Enter continue · /status");
}

#[test]
fn a_read_only_record_states_the_table_row() {
    let model = Model {
        database: Some("prod".to_owned()),
        read_only_record: true,
        ..Default::default()
    };

    assert_eq!(state(&model), State::ReadOnlyRecord);
    assert_eq!(title(state(&model)), "Message");
    assert_eq!(
        placeholder(&model),
        "This is a 0.5.0 session — your next message starts a new run"
    );
    assert_eq!(hint(&model, 100), "Enter send");
}

#[test]
fn ctrl_g_drops_the_queued_line_and_never_sends_it() {
    let mut model = queued_model();
    assert_eq!(model.queued(), Some("second"));

    let cmds = update(&mut model, ctrl_g());

    assert!(
        cmds.is_empty(),
        "Ctrl+G drops the held line; it does not send it: {cmds:?}"
    );
    assert_eq!(model.queued(), None, "the held line is gone");
    assert!(model.is_busy(), "the turn it was held behind keeps running");
    let after = render(&model);
    assert!(
        !after.contains("↑ edit queued"),
        "the queued state's hint is gone with the line: {after}"
    );
    assert!(
        after.contains("Enter queue · Ctrl+O detail · Esc stop turn"),
        "the composer is back to the running turn's row: {after}"
    );
}

#[test]
fn ctrl_g_with_nothing_queued_does_nothing() {
    let mut model = Model::default();
    typed(&mut model, "a draft");
    let before = render(&model);

    assert!(
        update(&mut model, ctrl_g()).is_empty(),
        "nothing is queued, so there is nothing to ask the runtime for"
    );
    assert_eq!(model.composer(), "a draft", "the draft is untouched");
    assert_eq!(render(&model), before, "the box does not change");

    typed(&mut model, "!");
    update(&mut model, enter());
    assert!(model.is_busy());
    assert!(
        update(&mut model, ctrl_g()).is_empty(),
        "a running turn with nothing held behind it has nothing to drop"
    );
    assert_eq!(
        model.toast(),
        None,
        "a press with nothing to drop is not said out loud"
    );
}

#[test]
fn a_multi_line_paste_is_one_event_that_lands_in_the_composer_intact() {
    let block = "select order_date,\n       count(*)\nfrom orders\ngroup by 1";
    let mut model = Model::default();

    let cmds = update(&mut model, Msg::Paste(block.to_owned()));

    assert!(
        cmds.is_empty(),
        "one paste is text, never a key press to act on: {cmds:?}"
    );
    assert_eq!(model.composer(), block, "every line lands, verbatim");
    assert!(!model.is_busy(), "a paste does not send anything");
    let drawn = render(&model);
    for line in block.lines() {
        assert!(
            drawn.contains(line),
            "pasted line {line:?} is drawn: {drawn}"
        );
    }
}

#[test]
fn a_waiting_call_disables_the_composer_and_names_the_keys_that_answer_it() {
    let mut model = Model {
        database: Some("demo".to_owned()),
        ..Default::default()
    };
    model.ask(crate::front::tui::view::cards_fixture::shell_question());

    assert_eq!(
        state(&model),
        State::Disabled,
        "a call waiting for an answer owns the surface"
    );
    assert!(
        title(state(&model)).contains("disabled"),
        "the box says it cannot be typed into: {}",
        title(state(&model))
    );
    assert_eq!(
        placeholder(&model),
        "Answer the approval above to type again",
        "and says what it is waiting for"
    );
    assert!(
        hint(&model, 100).contains("y allow"),
        "the hint row names the keys that answer: {}",
        hint(&model, 100)
    );
}

#[test]
fn a_waiting_call_names_only_the_answers_it_offers() {
    let mut model = Model {
        database: Some("demo".to_owned()),
        ..Default::default()
    };
    // A call with no token to grant: nothing could pre-answer it, so the card
    // offers no session answer — and the hint row must not offer one either.
    model.ask(crate::front::tui::view::cards_fixture::ungranted_question());

    let hint = hint(&model, 100);
    assert!(
        !hint.contains("s this session"),
        "a card with no grant offers no session answer, so the hint names none: {hint}"
    );
    assert_eq!(
        hint, "y allow once · n deny · Esc deny",
        "the hint row is the handful of answers the panel actually offers"
    );
}
