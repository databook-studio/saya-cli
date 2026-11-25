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

/// One request's panel: the figures the task names, in one bounded block.
#[test]
fn the_panel_states_requests_tokens_cost_and_context() {
    let mut usage = SessionUsage::default();
    let mut turn = TokenUsage::new(51_200, 10);
    turn.cached_input_tokens = Some(12_800);
    usage.answering.record_call(&turn);
    usage.count_request();
    usage.note_request(Some(51_200), Some(3_000));

    let panel = usage.panel(Some(200_000), Some(1_000_000));
    for word in [
        "Session usage — 1 request",
        "Prompt tokens: 51200",
        "Cached tokens: 12800",
        "Completion tokens: 10",
        "Cost: $0.003",
        "Context: 26% of 200000 tokens",
        "Ceiling: $1 — $0.003 spent",
    ] {
        assert!(panel.contains(word), "missing {word:?} in:\n{panel}");
    }
}

#[test]
fn the_panel_says_an_unreported_cost_never_trips_the_ceiling() {
    let mut usage = SessionUsage::default();
    usage.answering.record_call(&TokenUsage::new(10, 2));
    usage.count_request();
    usage.note_request(Some(10), None);

    let panel = usage.panel(Some(1_000), Some(1_000_000));
    assert!(
        panel.contains("Cost: not reported by the provider"),
        "{panel}"
    );
    assert!(panel.contains("cannot stop a turn"), "{panel}");
}

#[test]
fn the_panel_names_the_way_to_set_a_ceiling() {
    let usage = SessionUsage::default();
    let panel = usage.panel(None, None);
    assert!(
        panel.contains("Ceiling: none (set one with /usage --limit <usd>)"),
        "{panel}"
    );
    assert!(panel.contains("Context: no request yet"), "{panel}");
}

/// A ceiling stops the next request at the boundary — spend equal to the
/// ceiling is enough — and says the two ways forward.
#[test]
fn the_ceiling_stops_at_the_boundary() {
    assert_eq!(super::ceiling_stop(Some(999_999), Some(1_000_000)), None);
    let sentence = super::ceiling_stop(Some(1_000_000), Some(1_000_000))
        .expect("spend at the ceiling stops the next request");
    for word in [
        "Session cost ceiling reached",
        "/usage --limit <usd>",
        "start a new session",
    ] {
        assert!(sentence.contains(word), "missing {word:?} in {sentence}");
    }
    assert!(super::ceiling_stop(Some(1_500_000), Some(1_000_000)).is_some());
}

/// Unreported spend is unknown, not zero and not over: it never trips a
/// ceiling, and a session with no ceiling never stops.
#[test]
fn an_unreported_cost_never_stops_a_turn() {
    assert_eq!(super::ceiling_stop(None, Some(1_000_000)), None);
    assert_eq!(super::ceiling_stop(Some(1_000_000), None), None);
}

/// A provider that completes a request without reporting tokens still answered
/// a request: the header counts requests, not reported-usage turns, and the
/// token lines say the provider reported none rather than showing zeros.
#[test]
fn a_turn_without_reported_usage_is_still_a_request() {
    use crate::chat::ChatOutcome;
    use crate::front::TurnTotals;

    let mut usage = SessionUsage::default();
    TurnTotals::default().fold_into(&mut usage, &ChatOutcome::Answered);

    let panel = usage.panel(Some(200_000), None);
    assert!(
        panel.contains("Session usage — 1 request"),
        "a completed request is counted even with no usage report:\n{panel}"
    );
    assert!(
        !panel.contains("no requests yet"),
        "a completed request is not `no requests yet`:\n{panel}"
    );
    for line in [
        "Prompt tokens: not reported by the provider",
        "Completion tokens: not reported by the provider",
        "Cost: not reported by the provider",
    ] {
        assert!(panel.contains(line), "missing {line:?} in:\n{panel}");
    }
}
