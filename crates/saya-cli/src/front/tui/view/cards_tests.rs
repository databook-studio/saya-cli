//! The docked card's contract: what it states about a call, and how much of it
//! the screen shows.
//!
//! The one rule under test throughout is that the card states the call and
//! nothing else: the body is the question's facts byte for byte, so the
//! operator answers for exactly the bytes they read.

use super::cards::{BODY_ROWS, Kind, approval, clarification};
use super::cards_fixture::{
    ASK, SCREEN_ROWS, helper_question, long_question, plan_question, shell_question, sql_question,
};

#[test]
fn a_sql_call_shows_the_operators_facts_byte_for_byte() {
    let question = sql_question();

    let card = approval(&question, ASK, SCREEN_ROWS);

    assert_eq!(
        card.body().join("\n"),
        question.facts,
        "the body is the facts the operator was handed, line for line"
    );
    assert_eq!(
        card.body().join("\n"),
        "on: demo\nSELECT COUNT(*) AS orders_since_december\n  FROM orders\n WHERE order_date >= '2025-12-01'",
        "a query's facts are today's: the connection, then the statement as written"
    );
    assert_eq!(card.kind(), Kind::Approval);
    assert_eq!(
        card.body().len(),
        4,
        "four lines stated, four rows: {:?}",
        card.body()
    );
}

#[test]
fn a_shell_call_shows_the_operators_facts_byte_for_byte() {
    let question = shell_question();

    let card = approval(&question, ASK, SCREEN_ROWS);

    assert_eq!(
        card.body().join("\n"),
        question.facts,
        "the command and its directory, exactly as the operator was handed them"
    );
    assert_eq!(
        card.body().join("\n"),
        "in: ~/shop\n$ python3 -m unittest",
        "a command's facts are today's: the directory, then the command"
    );
}

#[test]
fn the_title_names_the_caller_and_a_helpers_question_names_the_helper() {
    let root = approval(&sql_question(), ASK, SCREEN_ROWS);
    assert_eq!(
        root.title(),
        "Approve? · Run a read-only query",
        "the title opens with the card's word and the caller's own line"
    );

    let helper = helper_question();
    let card = approval(&helper, ASK, SCREEN_ROWS);

    assert!(
        card.title().contains("helper-2"),
        "a helper's question names the helper: {}",
        card.title()
    );
    assert!(
        card.title().contains("job-a"),
        "and the job it works for: {}",
        card.title()
    );
    assert!(
        card.title().contains(&helper.caller),
        "the caller's line is stated, never re-worded: {}",
        card.title()
    );
}

#[test]
fn an_approval_says_why_it_is_asked_and_the_grant_a_yes_would_record() {
    let card = approval(&sql_question(), ASK, SCREEN_ROWS);

    assert_eq!(
        card.because(),
        Some("asked because: mode ask · no grant for sql:demo"),
        "the reason names the mode in force and the grant the call has none of"
    );
    assert_eq!(
        card.keys(),
        "y allow once   s allow sql:demo this session   n deny   v view",
        "today's answers, with the token a session-wide yes records"
    );
}

#[test]
fn eighty_by_twenty_four_caps_the_body_at_six_rows_and_offers_view_all() {
    let question = long_question();
    let card = approval(&question, ASK, SCREEN_ROWS);

    assert_eq!(
        card.body().len(),
        12,
        "the card holds the whole statement, whatever the screen"
    );
    assert_eq!(
        card.shown(false).len(),
        BODY_ROWS,
        "80x24 shows six body rows: {:?}",
        card.shown(false)
    );
    assert_eq!(
        card.shown(false),
        &card.body()[..6],
        "the visible rows are the body's first rows, verbatim"
    );
    assert_eq!(card.withheld(), 6, "and the cap withholds the other six");
    assert!(
        card.keys().contains("v view all"),
        "a clipped body says how to see all of it: {}",
        card.keys()
    );
    assert_eq!(
        card.shown(true),
        card.body(),
        "`v` shows the whole body, still verbatim"
    );
}

#[test]
fn the_plan_card_reads_the_plan_and_its_two_answers() {
    let question = plan_question();

    let card = approval(&question, ASK, SCREEN_ROWS);

    assert_eq!(
        card.kind(),
        Kind::Plan,
        "the plan gate's ask is the plan card, not an approval"
    );
    assert_eq!(
        card.body().join("\n"),
        question.facts,
        "the plan is the body, line for line"
    );
    assert_eq!(
        card.keys(),
        "y approve plan · n keep planning",
        "the plan has the two answers its decision has"
    );
    assert_eq!(
        card.because(),
        None,
        "no grant pre-answers a plan, so no reason line states one"
    );
}

#[test]
fn a_clarification_card_reads_the_question_and_offers_a_field() {
    let text =
        "Which revenue do you mean?\n  1. Gross order value (orders.total)\n  2. Net of refunds";

    let card = clarification(text, SCREEN_ROWS);

    assert_eq!(card.kind(), Kind::Question);
    assert_eq!(
        card.body().join("\n"),
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
