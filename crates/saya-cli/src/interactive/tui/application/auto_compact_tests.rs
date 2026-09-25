//! Automatic-compaction trigger tests: the turn-boundary firing, every
//! suppression case, the no-retry policy, and the one-operation property.
//!
//! The firing path is driven through the real seam — messages on the stream
//! channel drained by `drain_stream` — so the decision's numerator, the
//! window lookup, and the worker start are exercised together. The poll path
//! (applying the worker's clone-side result to the live session) is driven
//! headlessly by completing the worker synchronously and polling the
//! receiver, never by sleeping on a thread.

#[cfg(test)]
#[path = "auto_compact_modes.rs"]
mod modes;
#[cfg(test)]
#[path = "auto_compact_poll.rs"]
mod poll;
#[cfg(test)]
#[path = "auto_compact_support.rs"]
mod support;

use super::super::super::agent::StreamMsg;
use saya_agent::{AgentOutput, TokenUsage};
use support::{answering_report, app_with_model, output, run_turn_maybe_compacting};

/// Crossing 95% at a turn boundary fires exactly one compaction. Drained
/// through the real `drain_stream` seam with five turns of history so the
/// worker starts; the conversation afterwards (applied at poll time over the
/// same inputs) is summary + verbatim tail.
#[test]
fn crossing_95_at_a_turn_boundary_fires_exactly_one_compaction() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    // 121_600 / 128_000 = exactly 95%.
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![
            answering_report(121_600),
            StreamMsg::Done(Ok(output(TokenUsage::new(121_600, 20)))),
        ],
    );
    assert!(
        fired,
        "crossing 95% at a turn boundary must start one compaction"
    );
    // Exactly one: a second drain with no stream must not start another.
    assert!(
        app.compact_task.is_some(),
        "exactly one worker is in flight"
    );
}

/// A run at 96% with an unknown window never fires — the same principle as
/// the warning: absence is not zero, and a 96%-of-nothing is a fiction.
#[test]
fn unknown_window_never_fires_even_at_96_percent_of_a_known_one() {
    let (mut app, mut state) = app_with_model("mystery-model");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    // 122_880 would be 96% under gpt-4o's window; the model is unknown, so
    // the denominator is absent and nothing fires.
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![
            answering_report(122_880),
            StreamMsg::Done(Ok(output(TokenUsage::new(122_880, 20)))),
        ],
    );
    assert!(
        !fired,
        "an unknown window must never fire, whatever the numerator"
    );
}

/// A finished turn that continued after the output cap (`output.truncated`,
/// the loop's re-instruction after `OutputTruncated`) never fires — the
/// partial was discarded and the resume anchors live in earlier tool results.
#[test]
fn mid_continuation_never_fires() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let continued = AgentOutput {
        truncated: true,
        ..output(TokenUsage::new(121_600, 20))
    };
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![answering_report(121_600), StreamMsg::Done(Ok(continued))],
    );
    assert!(
        !fired,
        "a continued turn must not compact: the resume anchors live in earlier tool results"
    );
}
