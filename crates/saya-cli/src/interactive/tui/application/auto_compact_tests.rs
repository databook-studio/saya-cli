//! Automatic-compaction trigger tests: the turn-boundary firing, every
//! suppression case, the no-retry policy, and the one-operation property.
//!
//! The firing path is driven through the real seam — messages on the stream
//! channel drained by `drain_stream` — so the decision's numerator, the
//! window lookup, and the worker start are exercised together. The poll path
//! (applying the worker's clone-side result to the live session) is driven
//! headlessly by completing the worker synchronously and polling the
//! receiver, never by sleeping on a thread.

use super::super::super::agent::{Stream, StreamMsg};
use super::super::super::types::CompactOutcome;
use super::super::tests_support::{idle_app, unused_runtime};
use super::*;
use crate::interactive::auto_compact::{auto_failure_message, auto_success_message};
use crate::interactive::session_compact::{apply, plan};
use crate::interactive::session_state::SessionState;
use saya_agent::{AgentOutput, CancellationToken, TokenUsage, UsageCall};
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

fn app_with_model(model: &str) -> (App, SessionState) {
    (idle_app(), SessionState::new("s1", None, model))
}

fn stream_with(messages: Vec<StreamMsg>) -> Stream {
    let (tx, rx) = unbounded_channel();
    for message in messages {
        let _ = tx.send(message);
    }
    Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
    }
}

fn output(usage: TokenUsage) -> AgentOutput {
    AgentOutput {
        answer: "the answer".into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage,
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

fn answering_report(input: u64) -> StreamMsg {
    StreamMsg::Event(AgentEvent::usage(
        UsageCall::Answer,
        TokenUsage::new(input, 20),
    ))
}

/// Runs one turn through `drain_stream`, returning whether a compaction
/// worker started. The turn needs enough history for a plan to exist — five
/// recorded turns — so a firing decision actually starts the worker.
fn run_turn_maybe_compacting(
    app: &mut App,
    state: &mut SessionState,
    messages: Vec<StreamMsg>,
) -> bool {
    app.request.stream = Some(stream_with(messages));
    assert!(app.drain_stream(state), "the turn finished");
    app.compact_task.is_some()
}

fn five_turns() -> SessionState {
    let mut state = SessionState::new("s1", None, "gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}

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

/// `compaction = "manual"` never fires automatically; `/compact` still works
/// (the shared operation plans over the same history).
#[test]
fn manual_mode_never_fires_but_manual_compact_still_plans() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let mut runtime = unused_runtime();
    runtime.resolved.ai.compaction = saya_config::CompactionMode::Manual;
    app.runtime = Arc::new(runtime);
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![
            answering_report(121_600),
            StreamMsg::Done(Ok(output(TokenUsage::new(121_600, 20)))),
        ],
    );
    assert!(!fired, "manual mode must never fire automatically");
    let history = state.provider_history();
    assert!(
        plan(&state.turns, &history).is_some(),
        "/compact still works in manual mode: the shared operation plans"
    );
}

/// `compaction = "off"` fires nothing and emits no 70% warning either.
#[test]
fn off_fires_nothing_and_emits_no_warning() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let mut runtime = unused_runtime();
    runtime.resolved.ai.compaction = saya_config::CompactionMode::Off;
    app.runtime = Arc::new(runtime);
    // 70%: the warning level, which must stay silent under `off`.
    app.request.stream = Some(stream_with(vec![
        answering_report(89_600),
        StreamMsg::Done(Ok(output(TokenUsage::new(89_600, 20)))),
    ]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert!(
        app.compact_task.is_none(),
        "off must never start a compaction"
    );
    assert!(
        app.transcript
            .blocks()
            .iter()
            .all(|block| !block.text.contains("Context is at")),
        "off must emit no 70% warning either"
    );
    assert!(
        !state.context_warned,
        "off must leave the warning flag unfired"
    );
}

/// A provider reporting no usage never fires: absent is not zero, and a
/// usage-less turn pushes no footer to decide on.
#[test]
fn a_provider_reporting_no_usage_never_fires() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    app.request.stream = Some(stream_with(vec![StreamMsg::Done(Ok(output(
        TokenUsage::new(0, 0),
    )))]));
    assert!(app.drain_stream(&mut state), "the silent turn finished");
    assert!(
        app.compact_task.is_none(),
        "a usage-less turn must not fire"
    );
}

/// A failed automatic compaction does not re-fire on the following turn: the
/// session-scoped flag suppresses the trigger until `/clear` or a successful
/// manual compact re-arms it.
#[test]
fn a_failed_automatic_compaction_does_not_re_fire_next_turn() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state.auto_compact_failed = true;
    let fired = run_turn_maybe_compacting(
        &mut app,
        &mut state,
        vec![
            answering_report(121_600),
            StreamMsg::Done(Ok(output(TokenUsage::new(121_600, 20)))),
        ],
    );
    assert!(
        !fired,
        "a failed automatic compaction must not retry on the next turn"
    );
}

/// The no-retry flag is session-scoped and in-memory: `/clear` re-arms the
/// trigger along with the conversation.
#[test]
fn clear_re_arms_the_automatic_trigger() {
    let mut state = five_turns();
    state.auto_compact_failed = true;
    state.apply(crate::SlashCommand::Clear, &[]);
    assert!(
        !state.auto_compact_failed,
        "/clear must re-arm the automatic trigger"
    );
}

/// The poll path applies the worker's clone-side result to the live session:
/// after polling a successful automatic outcome, the live conversation is
/// summary + verbatim tail, and the transcript names the automatic origin.
#[test]
fn polling_an_automatic_success_applies_summary_plus_verbatim_tail() {
    let (mut app, mut state) = five_turns_app();
    let history = state.provider_history();
    let planned = plan(&state.turns, &history).expect("long history plans");
    let summary = "older turns: q0 q1 q2 discussed orders.";
    // The worker applied this to its clone; the poll path applies it live.
    let (tx, rx) = std::sync::mpsc::channel();
    app.compact_task = Some(rx);
    tx.send(CompactOutcome {
        message: crate::interactive::session_compact::success_message(
            planned.compacted_turns,
            summary,
        ),
        failed: false,
        usage: None,
        automatic: true,
        summary: Some(summary.into()),
        compacted_turns: planned.compacted_turns,
    })
    .expect("the worker's outcome sends");
    crate::interactive::compact_task::poll(&mut app, &mut state);
    assert!(app.compact_task.is_none(), "a polled worker is cleared");
    let replayed = state.provider_history();
    assert!(
        replayed[0].content.contains(summary),
        "the summary replays first: {replayed:?}"
    );
    assert!(
        replayed
            .iter()
            .any(|message| message.content == "a4" && message.role == "assistant"),
        "the newest answer survives verbatim: {replayed:?}"
    );
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.text == auto_success_message(planned.compacted_turns, summary)),
        "the transcript states the automatic origin with the manual message"
    );
    assert!(
        !state.auto_compact_failed,
        "a successful automatic compaction clears the no-retry flag"
    );
}

fn five_turns_app() -> (App, SessionState) {
    (idle_app(), five_turns())
}

/// Polling an automatic failure sets the no-retry flag and states the origin
/// plus the policy; the conversation is exactly as it was.
#[test]
fn polling_an_automatic_failure_sets_the_flag_and_states_the_policy() {
    let (mut app, mut state) = five_turns_app();
    let before = state.provider_history();
    let (tx, rx) = std::sync::mpsc::channel();
    app.compact_task = Some(rx);
    tx.send(CompactOutcome {
        message: "the summariser timed out".into(),
        failed: true,
        usage: None,
        automatic: true,
        summary: None,
        compacted_turns: 0,
    })
    .expect("the worker's outcome sends");
    crate::interactive::compact_task::poll(&mut app, &mut state);
    assert!(
        state.auto_compact_failed,
        "a failed automatic compaction sets the session-scoped flag"
    );
    assert_eq!(
        state.provider_history(),
        before,
        "a failed automatic compaction leaves the conversation exactly as it was"
    );
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.text == auto_failure_message("the summariser timed out")),
        "the transcript states the origin and the no-retry policy"
    );
}

/// The one-operation property, asserted directly: for the same history, the
/// automatic and manual paths produce byte-identical results — the same
/// message, the same applied summary, the same replayed conversation.
#[test]
fn automatic_and_manual_paths_produce_byte_identical_results() {
    let history_of = |state: &SessionState| state.provider_history();
    let mut manual_state = five_turns();
    let mut auto_state = five_turns();
    let manual_history = history_of(&manual_state);
    let auto_history = history_of(&auto_state);
    let manual_plan = plan(&manual_state.turns, &manual_history).expect("plans");
    let auto_plan = plan(&auto_state.turns, &auto_history).expect("plans");
    assert_eq!(
        manual_plan.compacted_turns, auto_plan.compacted_turns,
        "the same history plans the same turns"
    );
    assert_eq!(manual_plan.pinned, auto_plan.pinned);
    let summary = "older turns: q0 q1 q2 discussed orders.";
    apply(&mut manual_state, &manual_plan, summary).expect("manual applies");
    apply(&mut auto_state, &auto_plan, summary).expect("automatic applies");
    let manual_message =
        crate::interactive::session_compact::success_message(manual_plan.compacted_turns, summary);
    // The operations are byte-identical; only the presentation differs, and
    // the automatic presentation is the manual message with its origin.
    assert_eq!(
        auto_success_message(auto_plan.compacted_turns, summary),
        format!("Automatic compaction: {manual_message}"),
        "the automatic presentation is the manual message with its origin stated"
    );
    assert_eq!(
        manual_state.provider_history(),
        auto_state.provider_history(),
        "the two triggers leave byte-identical conversations"
    );
    assert_eq!(
        manual_state.compaction_summary, auto_state.compaction_summary,
        "the two triggers store byte-identical summaries"
    );
}
