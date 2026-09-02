//! Tests for [`SessionUsage`] — the session token accumulator.
//!
//! These tests pin the two invariants the `Option` fields exist for:
//! - Invariant 1 (deliverable 5): a cache hit rate over unreported data is
//!   "unknown", never 0%.
//! - Invariant 4 (deliverable 6): a usage-less turn adds nothing.

use super::SessionUsage;
use saya_agent::TokenUsage;

/// Helper: build a `TokenUsage` with only the two base counters set.
fn usage(input: u64, output: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        ..Default::default()
    }
}

/// Deliverable 5 — the test invariant 1 exists for. A session where no
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

/// The flip side of deliverable 5 and the thesis of `2e1c69a`: a *reported*
/// `Some(0)` is a cache miss, not an absence. It must render as "0%", never
/// "unknown". This is the case where the cache was cold and the provider said
/// so — conflating it with "the provider said nothing" erases the signal.
#[test]
fn reported_zero_cache_is_a_cold_cache_not_unknown() {
    let mut session = SessionUsage::default();
    session.record(&TokenUsage {
        input_tokens: 100,
        output_tokens: 50,
        cached_input_tokens: Some(0),
        ..Default::default()
    });
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
    session.record(&TokenUsage {
        input_tokens: 100,
        output_tokens: 50,
        cached_input_tokens: Some(80),
        reasoning_tokens: Some(30),
        ..Default::default()
    });
    session.record(&TokenUsage {
        input_tokens: 200,
        output_tokens: 80,
        cached_input_tokens: Some(20),
        ..Default::default()
    });
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

/// Q3 — the hit rate is `Σcached / Σinput`, a ratio of sums, not a mean of
/// per-turn rates. Two turns with different rates must produce the pooled
/// rate, not the average. Turn 1: 80/100 = 80%. Turn 2: 20/200 = 10%.
/// Mean of rates = 45%. Pooled = 100/300 = 33%. The test pins the pooled one.
#[test]
fn cache_hit_rate_is_pooled_not_averaged() {
    let mut session = SessionUsage::default();
    session.record(&TokenUsage {
        input_tokens: 100,
        output_tokens: 50,
        cached_input_tokens: Some(80),
        ..Default::default()
    });
    session.record(&TokenUsage {
        input_tokens: 200,
        output_tokens: 80,
        cached_input_tokens: Some(20),
        ..Default::default()
    });
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

/// The formula is stated in the output where the user can see it (Q3 /
/// deliverable 4), not just in the help text.
#[test]
fn render_states_the_hit_rate_formula() {
    let mut session = SessionUsage::default();
    session.record(&TokenUsage {
        input_tokens: 100,
        output_tokens: 50,
        cached_input_tokens: Some(90),
        ..Default::default()
    });
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

/// The extraction call's usage is labelled apart from the answering total. A
/// session that ran extraction reports a separate "Learning call" section; the
/// two totals must not be merged — the learning call is invisible in the
/// transcript, so nothing else would reveal the omission.
#[test]
fn extraction_usage_is_labelled_apart_from_the_answering_total() {
    let mut session = SessionUsage::default();
    session.record(&usage(300, 130));
    session.record_learning(Some(usage(50, 20)));
    let rendered = session.render();
    assert!(
        rendered.contains("Session token usage"),
        "answering section must be labelled, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Input tokens: 300"),
        "answering input total must be present, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Learning call"),
        "learning call must be labelled apart, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Input tokens: 50"),
        "learning input total must be present and distinct, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Input tokens: 350"),
        "learning and answering must not be merged into one total, got:\n{rendered}"
    );
}

/// A provider that reported nothing for extraction adds nothing and stays
/// distinguishable from one that reported zeros. `None` omits the learning
/// section entirely (absent is not zero); a reported zero shows the section
/// with zeros — the two must not collapse to the same display.
#[test]
fn extraction_that_reported_nothing_is_distinguishable_from_a_reported_zero() {
    let mut absent = SessionUsage::default();
    absent.record(&usage(100, 50));
    absent.record_learning(None);
    let absent_rendered = absent.render();

    let mut zero = SessionUsage::default();
    zero.record(&usage(100, 50));
    zero.record_learning(Some(usage(0, 0)));
    let zero_rendered = zero.render();

    assert!(
        !absent_rendered.contains("Learning call"),
        "absent extraction must not show a learning section, got:\n{absent_rendered}"
    );
    assert!(
        zero_rendered.contains("Learning call"),
        "a reported zero must show the learning section, got:\n{zero_rendered}"
    );
    assert_ne!(
        absent_rendered, zero_rendered,
        "absent and reported-zero must be distinguishable"
    );
    // The answering total is untouched in both.
    assert!(
        absent_rendered.contains("Input tokens: 100"),
        "answering total must be intact when extraction reported nothing, got:\n{absent_rendered}"
    );
    assert!(
        zero_rendered.contains("Input tokens: 100"),
        "answering total must be intact when extraction reported zero, got:\n{zero_rendered}"
    );
}

/// A timed-out extraction produced no response, so it reports no usage and
/// must not corrupt the totals: the answering total stays as recorded and the
/// learning section stays absent (no response means no number to report).
#[test]
fn a_timed_out_extraction_does_not_corrupt_the_totals() {
    let mut session = SessionUsage::default();
    session.record(&usage(300, 130));
    // A timeout yields no response, so the recorder is handed nothing.
    session.record_learning(None);
    let rendered = session.render();
    assert!(
        rendered.contains("Input tokens: 300"),
        "answering total must be intact, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Learning call"),
        "a timeout must not invent a learning section, got:\n{rendered}"
    );
}

/// With learning disabled no extraction call runs, so `/usage` must render
/// byte-for-byte what it rendered before this slice — no learning section
/// appears and the answering breakdown is untouched.
#[test]
fn with_learning_off_usage_output_is_unchanged() {
    let mut session = SessionUsage::default();
    session.record(&TokenUsage {
        input_tokens: 100,
        output_tokens: 50,
        cached_input_tokens: Some(80),
        ..Default::default()
    });
    let rendered = session.render();
    let expected = "Session token usage (1 turn):\n\n\
     \x20 Input tokens: 100\n\
     \x20 Output tokens: 50\n\
     \x20 Reasoning tokens: —\n\
     \x20 Cached input: 80\n\
     \x20 Cache creation: —\n\
     \x20 Cache hit rate: 80% (Σcached / Σinput)";
    assert_eq!(
        rendered, expected,
        "answering-only render must be byte-identical with learning off"
    );
}
