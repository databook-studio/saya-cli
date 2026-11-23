//! The docked card's contract: what it states about a call, and how much of it
//! the screen shows.
//!
//! The one rule under test throughout is that the card states the call and
//! nothing else: the rows an operator answers for are the question's facts byte
//! for byte, so they answer for exactly the bytes they read.
//!
//! What each family's title, block and choices are is
//! [`super::cards_panel_tests`]'s; this file is the body and the cap.

use super::cards::{BODY_ROWS, Kind, approval, clarification};
use super::cards_diff::Row;
use super::cards_fixture::{
    FOLDER, SCREEN_ROWS, helper_question, long_question, plan_question, shell_question,
    sql_question,
};

/// The rows' own text, for a test that compares them to the call's bytes.
fn texts(rows: Vec<Row>) -> Vec<String> {
    rows.into_iter().map(|row| row.text).collect()
}

#[test]
fn a_sql_call_shows_the_operators_facts_byte_for_byte() {
    let question = sql_question();

    let card = approval(&question, FOLDER, SCREEN_ROWS);

    assert_eq!(
        card.target().join("\n"),
        "SELECT COUNT(*) AS orders_since_december\n  FROM orders\n WHERE order_date >= '2025-12-01'",
        "a query's block is today's statement, as written"
    );
    assert_eq!(card.kind(), Kind::Approval);
    assert_eq!(
        texts(card.shown_rows(true, 96)).join("\n"),
        card.target().join("\n"),
        "what is shown joins back into the bytes that were said"
    );
}

#[test]
fn a_shell_call_shows_the_command_byte_for_byte() {
    let question = shell_question();

    let card = approval(&question, FOLDER, SCREEN_ROWS);

    assert_eq!(
        card.target().join("\n"),
        "$ python3 -m unittest",
        "the command is stated exactly as the call gave it"
    );
    assert!(
        !card.target().join("\n").contains("in: "),
        "and the directory it runs in is the card's place line, not its body"
    );
}

#[test]
fn the_title_names_a_helpers_question_as_the_helper() {
    let helper = helper_question();
    let card = approval(&helper, FOLDER, SCREEN_ROWS);

    assert_eq!(
        card.title(),
        "Allow this command?",
        "the panel says what is allowed; who asked is the caller's line below it"
    );
    assert!(
        helper.caller.contains("helper-2") && helper.caller.contains("job-a"),
        "the question still names the helper and its job: {}",
        helper.caller
    );
}

#[test]
fn eighty_by_twenty_four_caps_the_body_at_six_rows_and_offers_view_all() {
    let question = long_question();
    let card = approval(&question, FOLDER, SCREEN_ROWS);

    assert_eq!(
        card.target().len(),
        12,
        "the card holds the whole statement, whatever the screen"
    );
    assert_eq!(
        card.shown_rows(false, 96).len(),
        BODY_ROWS + 1,
        "80x24 shows six body rows and the count after them: {:?}",
        card.shown_rows(false, 96)
    );
    let folded = texts(card.shown_rows(false, 96));
    assert_eq!(
        folded[..BODY_ROWS],
        card.target()[..BODY_ROWS],
        "the visible rows are the body's first rows, verbatim"
    );
    assert_eq!(
        folded[BODY_ROWS], "+6 more lines",
        "and the cap says how many it is not showing: {folded:?}"
    );
    assert_eq!(card.withheld(96), 6, "`v` opens the six the cap left out");
    assert_eq!(
        texts(card.shown_rows(true, 96)),
        card.target(),
        "`v` shows the whole body, still verbatim"
    );
}

#[test]
fn the_plan_card_reads_the_plan_and_its_two_answers() {
    let question = plan_question();

    let card = approval(&question, FOLDER, SCREEN_ROWS);

    assert_eq!(
        card.kind(),
        Kind::Plan,
        "the plan gate's ask is the plan card, not an approval"
    );
    assert_eq!(
        card.target().join("\n"),
        question.facts,
        "the plan is the body, line for line"
    );
    assert_eq!(
        card.place(),
        None,
        "a plan runs nowhere in particular, so it names no place"
    );
}

#[test]
fn a_clarification_card_reads_the_question_and_offers_a_field() {
    let text =
        "Which revenue do you mean?\n  1. Gross order value (orders.total)\n  2. Net of refunds";

    let card = clarification(text, SCREEN_ROWS);

    assert_eq!(card.kind(), Kind::Question);
    assert_eq!(
        card.target().join("\n"),
        text,
        "the model's question is stated as it was asked"
    );
    assert_eq!(card.title(), "Question");
    assert_eq!(
        card.field(),
        Some("Type a number or your own answer"),
        "the answer is typed, so the card offers the field for it"
    );
}
