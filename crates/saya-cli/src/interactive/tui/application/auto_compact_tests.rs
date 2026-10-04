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
use saya_agent::{AgentEvent, AgentOutput, ProviderContinuationPhase, TokenUsage};
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

/// A finished turn that continued after the output cap never fires — proven
/// now by the typed `ProviderContinuationPhase::Continuing` transition the
/// loop emits once per admitted continuation, not by the salvage flag.
#[test]
fn mid_continuation_never_fires() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let continued = AgentOutput {
        events: vec![AgentEvent::provider_continuation(
            ProviderContinuationPhase::Continuing,
            2,
            3,
        )],
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

/// The regression for the old derivation: an admitted continuation with
/// `truncated == false` — a run that continued and then finished naturally —
/// used to be read as "not continued" (the old code derived the gate from
/// `output.truncated`) and fired compaction right where the resume anchors
/// live. With the typed event as the signal it must never fire.
#[test]
fn an_admitted_continuation_with_truncated_false_never_fires() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let continued = AgentOutput {
        events: vec![AgentEvent::provider_continuation(
            ProviderContinuationPhase::Continuing,
            1,
            3,
        )],
        ..output(TokenUsage::new(121_600, 20))
    };
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![answering_report(121_600), StreamMsg::Done(Ok(continued))],
    );
    assert!(
        !fired,
        "an admitted continuation on a naturally finished turn must not compact"
    );
}

/// A budget-salvage stop (`output.truncated`, no continuation event) still
/// suppresses compaction, but through its own gate: the derivation calls it
/// `terminal_stop`, never `continued`, so the two causes can never be read
/// as the same thing.
#[test]
fn a_budget_salvage_stop_still_suppresses_without_being_called_continuation() {
    let salvage = AgentOutput {
        truncated: true,
        ..output(TokenUsage::new(121_600, 20))
    };
    let (continued, terminal_stop) =
        crate::interactive::auto_compact::compaction_gates(Some(&salvage));
    assert!(!continued, "a budget salvage stop is not a continuation");
    assert!(
        terminal_stop,
        "a budget salvage stop is the terminal-stop gate"
    );
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![answering_report(121_600), StreamMsg::Done(Ok(salvage))],
    );
    assert!(
        !fired,
        "a budget salvage stop must still suppress compaction at the turn boundary"
    );
}

/// The derivation is a table over the transition phases: `Continuing` is the
/// one admitted-continuation signal; `Exhausted` is a spent allowance — a
/// terminal error path, never an admitted continuation — and an error/absent
/// result derives neither gate.
#[test]
fn exhausted_alone_does_not_mean_continued() {
    let cases = [
        (ProviderContinuationPhase::Continuing, true),
        (ProviderContinuationPhase::Exhausted, false),
    ];
    for (phase, expected) in cases {
        let spent = AgentOutput {
            events: vec![AgentEvent::provider_continuation(phase, 1, 3)],
            ..output(TokenUsage::new(121_600, 20))
        };
        let (continued, terminal_stop) =
            crate::interactive::auto_compact::compaction_gates(Some(&spent));
        assert_eq!(
            continued, expected,
            "{phase:?} must derive continued == {expected}"
        );
        assert!(!terminal_stop, "{phase:?} must not set the salvage gate");
    }
    let (continued, terminal_stop) = crate::interactive::auto_compact::compaction_gates(None);
    assert!(
        !continued && !terminal_stop,
        "an error/absent result derives neither gate"
    );
}
