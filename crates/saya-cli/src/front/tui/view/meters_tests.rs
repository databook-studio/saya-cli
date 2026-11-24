//! The meter segment: what it says when it knows, and what it shows when it does
//! not — nothing at all.
//!
//! Both absences are the contract. A window the launch may have resolved or not
//! is not a reason to invent one, and a provider that reported no cost is not a
//! reason to show `$0`.

use super::Meters;
use super::cost_text;

/// Both figures, as the row writes them.
#[test]
fn a_known_window_and_a_reported_cost_are_one_segment() {
    let meters = Meters {
        input_tokens: Some(51_200),
        window_tokens: Some(128_000),
        cost_micros: Some(31_200),
    };
    assert_eq!(meters.context_percent(), Some(40));
    assert_eq!(meters.segment().as_deref(), Some("ctx 40% · $0.0312"));
}

/// The context meter alone when the provider reports no cost.
#[test]
fn no_reported_cost_shows_no_money() {
    let meters = Meters {
        input_tokens: Some(64_000),
        window_tokens: Some(128_000),
        ..Meters::default()
    };
    assert_eq!(meters.segment().as_deref(), Some("ctx 50%"));
}

/// The cost alone when the window is unknown: an unknown window hides the
/// meter, it never guesses one.
#[test]
fn an_unknown_window_hides_the_meter_and_keeps_the_cost() {
    let meters = Meters {
        input_tokens: Some(51_200),
        cost_micros: Some(3_000),
        ..Meters::default()
    };
    assert_eq!(meters.context_percent(), None);
    assert_eq!(meters.segment().as_deref(), Some("$0.003"));
}

/// Nothing known is nothing shown.
#[test]
fn nothing_known_shows_nothing() {
    assert_eq!(Meters::default().segment(), None);
    assert_eq!(
        Meters {
            window_tokens: Some(128_000),
            ..Meters::default()
        }
        .segment(),
        None,
        "a window with no request against it is not a use"
    );
}

/// A reported zero cost is a reported zero: it reads `$0`, not nothing.
#[test]
fn a_reported_zero_cost_shows_zero() {
    let meters = Meters {
        cost_micros: Some(0),
        ..Meters::default()
    };
    assert_eq!(meters.segment().as_deref(), Some("$0"));
}

/// Money reads in dollars: whole ones, cents, and a fraction of a cent.
#[test]
fn cost_text_reads_as_money() {
    assert_eq!(cost_text(0), "$0");
    assert_eq!(cost_text(30_000), "$0.03");
    assert_eq!(cost_text(12_500), "$0.0125");
    assert_eq!(cost_text(1_000_000), "$1");
    assert_eq!(cost_text(1_500_000), "$1.5");
}
