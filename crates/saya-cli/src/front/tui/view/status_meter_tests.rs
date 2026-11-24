//! The status row's meter: the one short segment the row shows when the session
//! was told how full its window is and what it has spent — and nothing at all
//! when it was not.

use super::meters::Meters;
use super::status::{StatusFacts, status_line};
use super::status_fixture::{assert_row, full, metered, row};

/// A known window and a reported cost are one short segment at the end of the
/// row's left group: the share of the window the last request used, and what the
/// session has cost so far.
#[test]
fn a_known_window_and_a_reported_cost_are_one_segment() {
    let line = status_line(&metered(), 100);

    assert_row(
        &line,
        100,
        "ask │ deepseek-v4.1-flash │ demo +1 │ ⚠ data sharing on │ ctx 40% · $0.0312",
        "● working 6s · Esc stop",
    );
}

/// No reported cost: the meter keeps the context share and says no money — a
/// cost nobody reported is not `$0`.
#[test]
fn no_reported_cost_shows_no_money() {
    let facts = StatusFacts {
        meters: Meters {
            input_tokens: Some(64_000),
            window_tokens: Some(128_000),
            ..Meters::default()
        },
        ..full()
    };
    let line = status_line(&facts, 100);
    let text = row(&line);

    assert!(text.contains("│ ctx 50%"), "the share is there: {text:?}");
    assert!(!text.contains('$'), "no money without a report: {text:?}");
}

/// An unknown window hides the context share and nothing else: the cost the
/// provider did report still shows.
#[test]
fn an_unknown_window_hides_only_the_context_share() {
    let facts = StatusFacts {
        meters: Meters {
            input_tokens: Some(51_200),
            cost_micros: Some(3_000),
            ..Meters::default()
        },
        ..full()
    };
    let line = status_line(&facts, 100);
    let text = row(&line);

    assert!(!text.contains("ctx"), "no window, no share: {text:?}");
    assert!(text.contains("│ $0.003"), "the cost stays: {text:?}");
}

/// The meter is the first thing the row gives up: at 60 columns the mode, the
/// model, the database, the caveat and the help key all stay, and the meter is
/// gone.
#[test]
fn the_meter_is_dropped_first_when_the_row_is_narrow() {
    let line = status_line(&metered(), 60);

    assert_row(
        &line,
        60,
        "ask │ deepseek-v4.1-flash │ +1 │ ⚠ sharing",
        "● working 6s",
    );
    let text = row(&line);
    assert!(!text.contains("ctx"), "the meter went first: {text:?}");
    assert!(!text.contains('$'), "and its cost went with it: {text:?}");
    assert!(text.contains("? help") || text.contains("working"));
}

/// With nothing reported the row is exactly the calm row it was: no empty
/// meter, no placeholder, no `$0`.
#[test]
fn nothing_reported_leaves_the_row_untouched() {
    let line = status_line(&full(), 100);

    assert_row(
        &line,
        100,
        "ask │ deepseek-v4.1-flash │ demo +1 │ ⚠ data sharing on",
        "● working 6s · Esc stop",
    );
    assert!(
        !row(&line).contains("ctx") && !row(&line).contains('$'),
        "no meter is invented: {:?}",
        row(&line)
    );
}
