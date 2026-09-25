/// Suppression modes: manual and off never fire, a usage-less turn never fires, failures never retry.
/// Moved byte-identical from the hub; no snapshots involved.
use super::super::super::super::agent::StreamMsg;
use super::super::super::tests_support::unused_runtime;
use super::support::{
    answering_report, app_with_model, five_turns, output, run_turn_maybe_compacting, stream_with,
};
use crate::interactive::session_compact::plan;
use saya_agent::TokenUsage;
use std::sync::Arc;

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
