//! The transcript's rhythm (`02` §2): one blank row between turns, and the
//! operator's message visibly distinct from the answer under it.

use super::transcript_fixture;
use super::transcript_fixture::{ELAPSED, rendered};
use super::transcript_rows;

#[test]
fn turns_are_separated_by_one_blank_row() {
    let one = transcript_rows(
        &[transcript_fixture::database_turn(false)],
        &transcript_fixture::facts(ELAPSED),
        100,
        60,
    );
    let two = transcript_rows(
        &[
            transcript_fixture::database_turn(false),
            transcript_fixture::database_turn(false),
        ],
        &transcript_fixture::facts(ELAPSED),
        100,
        60,
    );

    assert_eq!(
        two.len(),
        one.len() * 2 + 1,
        "one blank row joins the two turns"
    );
    let rows = rendered(&two);
    let boundary = &rows[one.len() - 1..=one.len()];
    assert_eq!(
        boundary[1], "",
        "the turn ends, a blank row, then the next turn: {boundary:?}"
    );
}

#[test]
fn the_operator_s_message_carries_a_left_bar_and_the_answer_does_not() {
    let rows = rendered(&transcript_rows(
        &[transcript_fixture::database_turn(false)],
        &transcript_fixture::facts(ELAPSED),
        100,
        30,
    ));

    assert!(
        rows[0].starts_with("  │ ❯ "),
        "the message opens behind a left bar: {:?}",
        rows[0]
    );
    assert!(
        rows.iter()
            .skip(1)
            .any(|row| row.contains("110 orders since December")),
        "the answer is on screen: {rows:?}"
    );
    for row in rows.iter().skip(1) {
        assert!(
            !row.contains('❯'),
            "only the operator's words carry the prompt: {row:?}"
        );
    }
}

#[test]
fn the_bar_is_seven_bit_under_ascii() {
    let facts = super::transcript::TranscriptFacts {
        charset: crate::front::tui::theme::Charset::Ascii,
        ..transcript_fixture::facts(ELAPSED)
    };
    let rows = rendered(&transcript_rows(
        &[transcript_fixture::database_turn(false)],
        &facts,
        100,
        30,
    ));
    assert!(
        rows[0].starts_with("  | > "),
        "the bar and the prompt fall back whole: {:?}",
        rows[0]
    );
}
