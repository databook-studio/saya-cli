//! The session's counters beside the sums `/usage` renders: the last answering
//! request's input, which is what the context meter divides by, and the cost the
//! provider reported, which is what the session has spent.
//!
//! Both are reported figures. A provider that reported no cost leaves the spend
//! unknown, and a reported zero is a reported zero.

use super::SessionUsage;
use saya_agent::TokenUsage;

fn reported(input: u64, cost: Option<u64>) -> TokenUsage {
    let mut usage = TokenUsage::new(input, 10);
    usage.cost_micros = cost;
    usage
}

/// The last request's input is the last call's, never the sum: a turn that
/// made two calls sent the second request's prompt, not both prompts.
#[test]
fn the_last_request_is_the_last_call() {
    let mut usage = SessionUsage::default();
    usage.answering.record_call(&reported(1_000, Some(500)));
    usage.answering.record_call(&reported(51_200, Some(2_000)));
    usage.note_request(Some(51_200), Some(2_000));

    assert_eq!(usage.last_input_tokens(), Some(51_200));
    assert_eq!(
        usage.answering.input_tokens, 52_200,
        "the sum is still a sum"
    );
    assert_eq!(usage.cost_micros(), Some(2_000));
}

/// The spend is the sum of what was reported, and unknown until something is.
#[test]
fn cost_sums_only_what_was_reported() {
    let mut usage = SessionUsage::default();
    assert_eq!(usage.cost_micros(), None, "nothing reported is not zero");

    usage.note_request(Some(1_000), None);
    assert_eq!(
        usage.cost_micros(),
        None,
        "a turn that reported no cost leaves the spend unknown"
    );

    usage.note_request(Some(2_000), Some(1_500));
    usage.note_request(Some(3_000), Some(2_500));
    assert_eq!(usage.cost_micros(), Some(4_000), "reported costs add up");
}

/// A reported zero is a report: the spend is known to be nothing, which is not
/// the same as nobody having said.
#[test]
fn a_reported_zero_cost_is_known() {
    let mut usage = SessionUsage::default();
    usage.note_request(Some(10), Some(0));
    assert_eq!(usage.cost_micros(), Some(0));
}

/// A turn that reported nothing leaves the last request stated: silence is not
/// a measurement of zero either.
#[test]
fn an_unmeasured_turn_leaves_the_last_request_stated() {
    let mut usage = SessionUsage::default();
    usage.note_request(Some(1_000), Some(1_000));
    usage.note_request(None, None);

    assert_eq!(usage.last_input_tokens(), Some(1_000));
    assert_eq!(usage.cost_micros(), Some(1_000));
}

/// The extraction call's spend is not the answering spend: the meter counts the
/// calls the operator's question paid for.
#[test]
fn the_learning_call_is_a_separate_counter() {
    let mut usage = SessionUsage::default();
    usage.note_request(Some(1_000), Some(1_000));
    usage.record_learning(Some(reported(500, Some(400))));

    assert_eq!(usage.cost_micros(), Some(1_000));
    assert_eq!(usage.last_input_tokens(), Some(1_000));
}
