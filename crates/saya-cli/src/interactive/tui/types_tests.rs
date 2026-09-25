//! Tests for [`SessionUsage`] — the session token accumulator.
//!
//! These tests pin the two invariants the `Option` fields exist for:
//! - a cache hit rate over unreported data is
//!   "unknown", never 0%.
//! - a usage-less turn adds nothing.

use super::SessionUsage;
use saya_agent::TokenUsage;

#[cfg(test)]
#[path = "types_support.rs"]
mod support;

use support::usage;

/// A session where no
/// provider reported cached tokens must render the hit rate as "unknown", not
/// "0%". This is the most skippable test in the slice and the reason the
/// `Option` fields exist at all: a display layer that renders `None` as 0%
/// throws away the distinction `2e1c69a` built the type for.
#[test]
fn cache_hit_rate_is_unknown_when_no_turn_reported_cached_tokens() {
    let mut session = SessionUsage::default();
    session.record(&usage(100, 50));
    session.record(&usage(200, 80));
    let rendered = session.render();
    assert!(
        rendered.contains("unknown"),
        "cache hit rate over unreported data must be unknown, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("0%"),
        "must not invent 0% for unreported cache data, got:\n{rendered}"
    );
}

/// The flip side, and the thesis of `2e1c69a`: a *reported*
/// `Some(0)` is a cache miss, not an absence. It must render as "0%", never
/// "unknown". This is the case where the cache was cold and the provider said
/// so — conflating it with "the provider said nothing" erases the signal.
#[test]
fn reported_zero_cache_is_a_cold_cache_not_unknown() {
    let mut session = SessionUsage::default();
    session.record(&TokenUsage::new(100, 50).with_cached_input(Some(0)));
    let rendered = session.render();
    assert!(
        rendered.contains("0%"),
        "a reported cache miss (Some(0)) must render 0%, not unknown, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("unknown"),
        "Some(0) is a report of zero, not absence — must not render unknown, got:\n{rendered}"
    );
}

/// Deliverable 6 — totals accumulate across turns, and a turn that reports no
/// usage (both base counters zero) adds nothing: it does not increment the
/// turn count and does not change the totals.
#[test]
fn totals_accumulate_across_turns_and_a_usage_less_turn_adds_nothing() {
    let mut session = SessionUsage::default();
    session.record(
        &TokenUsage::new(100, 50)
            .with_cached_input(Some(80))
            .with_reasoning(Some(30)),
    );
    session.record(&TokenUsage::new(200, 80).with_cached_input(Some(20)));
    assert_eq!(session.answering.input_tokens, 300);
    assert_eq!(session.answering.output_tokens, 130);
    assert_eq!(session.answering.cached_input_tokens, 100);
    assert_eq!(session.answering.reasoning_tokens, 30);
    assert_eq!(session.answering.turns, 2);

    // A turn that reports nothing (both counters zero) adds nothing.
    session.record(&TokenUsage::default());
    assert_eq!(session.answering.input_tokens, 300);
    assert_eq!(session.answering.output_tokens, 130);
    assert_eq!(
        session.answering.turns, 2,
        "a usage-less turn must not increment the turn count"
    );
}

/// The hit rate is `Σcached / Σinput`, a ratio of sums, not a mean of
/// per-turn rates. Two turns with different rates must produce the pooled
/// rate, not the average. Turn 1: 80/100 = 80%. Turn 2: 20/200 = 10%.
/// Mean of rates = 45%. Pooled = 100/300 = 33%. The test pins the pooled one.
#[test]
fn cache_hit_rate_is_pooled_not_averaged() {
    let mut session = SessionUsage::default();
    session.record(&TokenUsage::new(100, 50).with_cached_input(Some(80)));
    session.record(&TokenUsage::new(200, 80).with_cached_input(Some(20)));
    let rendered = session.render();
    // 100 cached / 300 input = 33%, not 45% (mean of 80% and 10%).
    assert!(
        rendered.contains("33%"),
        "hit rate must be Σcached/Σinput = 33%, not the mean 45%, got:\n{rendered}"
    );
}

/// Fields the provider never reported render as `—`, not as 0 or "unknown".
/// The hit rate is the only field that says "unknown"; the counts say `—`.
#[test]
fn unreported_fields_render_as_dash() {
    let mut session = SessionUsage::default();
    session.record(&usage(100, 50));
    let rendered = session.render();
    assert!(
        rendered.contains("Reasoning tokens: —"),
        "unreported reasoning must render as —, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Cached input: —"),
        "unreported cached input must render as —, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Cache creation: —"),
        "unreported cache creation must render as —, got:\n{rendered}"
    );
}

/// The formula is stated in the output where the user can see it (
/// in the output), not just in the help text.
#[test]
fn render_states_the_hit_rate_formula() {
    let mut session = SessionUsage::default();
    session.record(&TokenUsage::new(100, 50).with_cached_input(Some(90)));
    let rendered = session.render();
    assert!(
        rendered.contains("Σcached / Σinput"),
        "the breakdown must state the formula, got:\n{rendered}"
    );
}

/// An empty session (no turns with usage) renders a clear message, not a row
/// of zeros.
#[test]
fn empty_session_renders_nothing_reported() {
    let session = SessionUsage::default();
    let rendered = session.render();
    assert!(
        rendered.contains("No token usage reported yet"),
        "empty session must say no usage was reported, got:\n{rendered}"
    );
}
