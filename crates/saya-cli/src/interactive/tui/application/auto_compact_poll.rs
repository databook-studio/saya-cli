use super::super::super::super::types::App;
use super::super::super::super::types::CompactOutcome;
use super::super::super::tests_support::idle_app;
/// Poll path and one-operation property: worker results apply live; auto and manual agree byte-for-byte.
/// Moved byte-identical from the hub; no snapshots involved.
use super::support::five_turns;
use crate::interactive::session::continuation::compaction::automatic::{
    auto_failure_message, auto_success_message,
};
use crate::interactive::session::continuation::compaction::manual::{apply, plan};
use crate::interactive::session::state::SessionState;

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
        message: crate::interactive::session::continuation::compaction::manual::success_message(
            planned.compacted_turns,
            summary,
        ),
        failed: false,
        usage: None,
        automatic: true,
        summary: Some(summary.into()),
        compacted_turns: planned.compacted_turns,
        source_identity: Some(planned.source_identity),
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
        source_identity: None,
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

/// A manual worker's clone-side success is rejected if the live plan changed
/// while it ran; the transcript reports that no compaction was applied.
#[test]
fn polling_a_manual_success_with_a_stale_plan_reports_failure() {
    let (mut app, mut state) = five_turns_app();
    let original_plan = plan(&state.turns, &state.provider_history()).expect("long history plans");
    let summary = "older turns: q0 q1 q2 discussed orders.";
    state.record_turn("q5", "a5", false, Vec::new());
    let live_history = state.provider_history();
    let success = crate::interactive::session::continuation::compaction::manual::success_message(
        original_plan.compacted_turns,
        summary,
    );
    let (tx, rx) = std::sync::mpsc::channel();
    app.compact_task = Some(rx);
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("compacting".into());
    tx.send(CompactOutcome {
        message: success.clone(),
        failed: false,
        usage: Some(saya_agent::TokenUsage::new(40, 10)),
        automatic: false,
        summary: Some(summary.into()),
        compacted_turns: original_plan.compacted_turns,
        source_identity: Some(original_plan.source_identity),
    })
    .expect("the worker's outcome sends");

    crate::interactive::compact_task::poll(&mut app, &mut state);

    assert_eq!(
        state.provider_history(),
        live_history,
        "stale work leaves live history untouched"
    );
    assert!(
        state.compaction_summary.is_none(),
        "stale work stores no summary"
    );
    assert!(
        app.compact_task.is_none(),
        "the completed worker is cleared"
    );
    assert!(app.request.started.is_none() && app.request.activity.is_none());
    assert_eq!(
        state.usage.learning.turns, 1,
        "worker usage remains accounted"
    );
    let blocks = app.transcript.blocks();
    assert!(
        blocks.iter().any(|block| block.kind
            == crate::interactive::tui::transcript::BlockKind::Error
            && block.text
                == crate::interactive::session::continuation::compaction::manual::failure_message(
                    "the conversation changed while compacting"
                )),
        "stale work reports an unapplied compaction as failure: {blocks:?}"
    );
    assert!(
        !blocks.iter().any(|block| block.text == success),
        "stale work never announces clone-side success: {blocks:?}"
    );
}

#[path = "auto_compact_poll_same_count_tests.rs"]
mod same_count;

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
        crate::interactive::session::continuation::compaction::manual::success_message(
            manual_plan.compacted_turns,
            summary,
        );
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
