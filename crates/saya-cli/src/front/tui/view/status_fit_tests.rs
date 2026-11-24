//! The drop order itself: what gives way first, what gives way in words only,
//! and what never gives way at all.

use saya_agent::PermissionMode;

use super::status::{StatusFacts, status_line};
use super::status_fixture::{assert_row, cells, full, row};

/// The model is not a candidate: a row too narrow for the three facts overflows
/// rather than forget which model answers, and the database keeps its extra
/// count until the words after it have gone.
#[test]
fn the_model_never_gives_way() {
    let line = status_line(&full(), 24);

    let text = row(&line);
    assert!(
        text.starts_with("ask │ deepseek-v4.1-flash"),
        "the mode and model survive every width: {text:?}"
    );
    assert!(
        text.contains("⚠ sharing"),
        "the sharing caveat shortens but stays: {text:?}"
    );
    assert!(
        cells(&line) > 24,
        "the row overflows before it hides a fact: {text:?}"
    );
}

/// The extra database count is the first thing to go: the primary keeps its
/// name whatever the width.
#[test]
fn the_extra_count_goes_before_the_name() {
    let line = status_line(&full(), 52);

    assert!(
        row(&line).starts_with("ask │ deepseek-v4.1-flash │ +1 │"),
        "the count gave way to its shorter form: {:?}",
        row(&line)
    );
    assert!(
        !row(&line).contains("demo"),
        "the extra database is counted, not named, on a narrow row: {:?}",
        row(&line)
    );
}

/// One database has no shorter form, so its step leaves it saying its name
/// while everything droppable around it is gone.
#[test]
fn a_single_database_is_never_removed() {
    let facts = StatusFacts {
        databases: vec!["demo".to_owned()],
        model: "m".to_owned(),
        sharing: true,
        ..StatusFacts::default()
    };
    let line = status_line(&facts, 34);

    assert_row(&line, 34, "ask │ m │ demo │ ⚠ sharing", "? help");
}

/// The right side is a state word with a hint, and the hint is what goes
/// first: the queued count stays when its key does not.
#[test]
fn the_hint_goes_before_the_state_word() {
    let facts = StatusFacts {
        activity: super::status::ActivityState::Queued { count: 1 },
        mode: PermissionMode::Plan,
        ..StatusFacts::default()
    };
    let wide = status_line(&facts, 60);
    assert!(
        row(&wide).ends_with("queued 1 · Ctrl+G drop"),
        "the hint rides beside the state word while there is room: {:?}",
        row(&wide)
    );

    let facts = StatusFacts {
        model: "deepseek-v4.1-flash".to_owned(),
        ..facts
    };
    let narrow = status_line(&facts, 60);

    let text = row(&narrow);
    assert!(
        text.ends_with("queued 1"),
        "the state word outlives its hint: {text:?}"
    );
    assert!(
        !text.contains("Ctrl+G drop"),
        "the hint is dropped first: {text:?}"
    );
}
