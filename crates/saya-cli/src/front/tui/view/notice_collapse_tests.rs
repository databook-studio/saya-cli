//! Repeated identical notices collapse into one row with a count.

use super::transcript_fixture::{ELAPSED, rendered};
use super::transcript_rows;
use crate::front::view::{Entry, Notice, TurnView};

/// One turn holding `messages` as notices, in order.
fn notices(messages: &[&str]) -> TurnView {
    let mut turn = TurnView::default();
    for message in messages {
        turn.push(Entry::Notice(Notice {
            message: (*message).to_owned(),
        }));
    }
    turn
}

/// The rows one turn of notices paints at 100 columns.
fn rows(messages: &[&str]) -> Vec<String> {
    rendered(&transcript_rows(
        &[notices(messages)],
        &super::transcript_fixture::facts(ELAPSED),
        100,
        30,
    ))
}

#[test]
fn three_identical_notices_are_said_once_with_a_count() {
    let rows = rows(&["mode: plan", "mode: plan", "mode: plan"]);
    let said: Vec<&String> = rows
        .iter()
        .filter(|row| row.contains("mode: plan"))
        .collect();

    assert_eq!(
        said.len(),
        1,
        "one line for the same notice three times: {rows:#?}"
    );
    assert!(
        said[0].contains("×3"),
        "the count says how often it was said: {:?}",
        said[0]
    );
}

#[test]
fn two_different_notices_each_keep_their_line() {
    let rows = rows(&["mode: plan", "workspace trusted", "mode: plan"]);
    assert_eq!(
        rows.iter().filter(|row| row.contains("mode: plan")).count(),
        2,
        "notices that are not adjacent do not collapse: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("workspace trusted")),
        "the other notice stays: {rows:#?}"
    );
}

#[test]
fn the_count_is_seven_bit_under_ascii() {
    let mut facts = super::transcript_fixture::facts(ELAPSED);
    facts.charset = crate::front::tui::theme::Charset::Ascii;
    let rows = rendered(&transcript_rows(
        &[notices(&["mode: plan", "mode: plan", "mode: plan"])],
        &facts,
        100,
        30,
    ));
    let said = rows
        .iter()
        .find(|row| row.contains("mode: plan"))
        .expect("the notice paints");
    assert!(
        said.contains("x3"),
        "the ASCII fallback counts in seven bits: {said:?}"
    );
}

#[test]
fn a_single_notice_carries_no_count() {
    let rows = rows(&["mode: plan"]);
    let said = rows
        .iter()
        .find(|row| row.contains("mode: plan"))
        .expect("the notice paints");
    assert!(
        !said.contains('×') && !said.contains("x3"),
        "a notice said once is not counted: {said:?}"
    );
}
