//! The headless decision table: the numerator is the LAST answering
//! request's input tokens and the shared predicate decides — rows (a) fires,
//! rows (b)–(h) each suppress for their own reason.

use super::super::headless_auto_compact_input;
use crate::interactive::session::continuation::compaction::automatic::should_auto_compact;
use crate::interactive::session::state::SessionState;
use saya_agent::{AgentEvent, ProviderContinuationPhase, TokenUsage};

use super::{answering_report, output, runtime_with_window};

/// The decision table over the headless input derivation and the shared
/// predicate: row (a) fires; rows (b)–(h) each suppress for their own reason.
#[test]
fn the_headless_decision_table_matches_the_shared_predicate() {
    let runtime = runtime_with_window(Some(1000));
    let state = SessionState::new("c43a", None, "test-model");

    // (a) The last Answer usage 960 of an explicit window 1000 is 96%: fires.
    let fires = output(vec![answering_report(960)], TokenUsage::new(960, 20), false);
    let input = headless_auto_compact_input(&runtime, &state, &fires);
    assert_eq!(
        input.answering_input,
        Some(960),
        "(a) the numerator rides the input"
    );
    assert_eq!(
        input.window,
        Some(1000),
        "(a) the explicit window rides the input"
    );
    assert!(
        !input.continued && !input.terminal_stop && !input.auto_failed,
        "(a) a normally completed turn carries no suppression gate"
    );
    assert!(
        should_auto_compact(&input),
        "(a) 960 of 1000 at the last answering request must fire"
    );

    // (b) The same turn shape with a larger SUMMED usage and a smaller last
    // request: the numerator is the LAST request's input, so 400 of 1000 is
    // 40% and never fires — occupancy is not the paid total.
    let summed = output(
        vec![answering_report(400)],
        TokenUsage::new(5000, 20),
        false,
    );
    let input = headless_auto_compact_input(&runtime, &state, &summed);
    assert_eq!(
        input.answering_input,
        Some(400),
        "(b) the numerator is the last request's input, not the summed run usage"
    );
    assert!(
        !should_auto_compact(&input),
        "(b) 400 of 1000 (the last request) is 40%: the summed 5000 must never decide"
    );

    // (c) A failed attempt's huge report is never the numerator either: only
    // a successful Answer report measures occupancy.
    let mixed = output(
        vec![
            AgentEvent::failed_attempt_usage(TokenUsage::new(120_000, 8)),
            answering_report(400),
        ],
        TokenUsage::new(120_400, 28),
        false,
    );
    let input = headless_auto_compact_input(&runtime, &state, &mixed);
    assert_eq!(
        input.answering_input,
        Some(400),
        "(c) a FailedAttemptUsage never becomes the numerator"
    );
    assert!(
        !should_auto_compact(&input),
        "(c) the failed attempt's huge input must not fire"
    );

    // (d) No Answer usage event at all: the numerator is absent, not zero.
    let silent = output(
        vec![AgentEvent::failed_attempt_usage(TokenUsage::new(
            120_000, 8,
        ))],
        TokenUsage::new(120_000, 8),
        false,
    );
    let input = headless_auto_compact_input(&runtime, &state, &silent);
    assert_eq!(input.answering_input, None, "(d) absent is not zero");
    assert!(
        !should_auto_compact(&input),
        "(d) no answering report never fires"
    );

    // (e) Unknown window: no percentage, no fire — 96% of nothing is a fiction.
    let unknown = runtime_with_window(None);
    let input = headless_auto_compact_input(&unknown, &state, &fires);
    assert_eq!(input.window, None, "(e) the unknown window stays absent");
    assert!(
        !should_auto_compact(&input),
        "(e) an unknown window never fires, whatever the numerator"
    );

    // (f) An admitted continuation (typed Continuing, truncated false): the
    // resume anchors live in earlier tool results, so this never compacts.
    let continued = output(
        vec![
            AgentEvent::provider_continuation(ProviderContinuationPhase::Continuing, 1, 3),
            answering_report(960),
        ],
        TokenUsage::new(960, 20),
        false,
    );
    let input = headless_auto_compact_input(&runtime, &state, &continued);
    assert!(
        input.continued,
        "(f) the typed transition is the continued gate"
    );
    assert!(
        !input.terminal_stop,
        "(f) a continuation is never read as a salvage stop"
    );
    assert!(
        !should_auto_compact(&input),
        "(f) a continued turn never compacts"
    );

    // (g) A budget-salvage stop: suppressed through its own gate, named apart
    // from the continuation.
    let salvage = output(vec![answering_report(960)], TokenUsage::new(960, 20), true);
    let input = headless_auto_compact_input(&runtime, &state, &salvage);
    assert!(
        input.terminal_stop,
        "(g) the salvage flag is the terminal-stop gate"
    );
    assert!(
        !input.continued,
        "(g) a salvage stop is never read as a continuation"
    );
    assert!(
        !should_auto_compact(&input),
        "(g) a salvaged turn never compacts"
    );

    // (h) A prior automatic failure this session latches the trigger off.
    let mut latched = SessionState::new("c43a-latched", None, "test-model");
    latched.auto_compact_failed = true;
    let input = headless_auto_compact_input(&runtime, &latched, &fires);
    assert!(input.auto_failed, "(h) the session latch rides the input");
    assert!(
        !should_auto_compact(&input),
        "(h) a latched session never fires again"
    );
}
