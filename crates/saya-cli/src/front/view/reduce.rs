//! The reducer: one session event folded into the transcript view.
//!
//! Pure — no I/O, no clock, no terminal — so a turn's whole layout is a
//! function of the events it received, and a test can fold a recorded stream
//! without a TUI.

use saya_agent::AgentEvent;
use serde_json::Value;

use super::entry::{
    Activity, Answer, Entry, Notice, ResultPreview, Step, StepState, TranscriptView, TurnView,
};
use crate::chat::ChatEvent;
use crate::render::tool_groups::is_failure_summary;

/// Folds one session event into the transcript.
pub(crate) fn apply(view: &mut TranscriptView, event: &ChatEvent) {
    match event {
        ChatEvent::Agent(agent) => apply_agent(view.current(), agent),
        ChatEvent::QueryCaptured(capture) => view
            .current()
            .push(Entry::ResultPreview(ResultPreview::from_capture(capture))),
        ChatEvent::Notice { message } => view.current().push(Entry::Notice(Notice {
            message: message.clone(),
        })),
        ChatEvent::TurnEnded(_) => end_turn(view.current()),
        // A prepared run and a call put to the operator are read by the status
        // line and the approval card, not by the transcript.
        ChatEvent::Ready { .. } | ChatEvent::Question(_) => {}
    }
}

/// Folds one event of the engine's own loop.
fn apply_agent(turn: &mut TurnView, event: &AgentEvent) {
    match event {
        AgentEvent::ToolRequested {
            name, arguments, ..
        } => requested(turn, name, arguments),
        AgentEvent::ToolCompleted { name, summary } => completed(turn, name, summary),
        AgentEvent::ToolDenied { name, reason, .. } => denied(turn, name, reason),
        AgentEvent::AssistantText { text } => answer(turn).text.push_str(text),
        // The attempt that produced the text so far was discarded, and the
        // retried stream re-emits the whole answer as fresh deltas.
        AgentEvent::TurnReset => {
            if let Some(answer) = turn.answer_mut() {
                answer.text.clear();
            }
        }
        // Metrics, recovery and knowledge events are progress and evidence;
        // the status line and the inspectors read them, not the turn's blocks.
        _ => {}
    }
}

/// The turn's activity block, opened by the turn's first tool call.
///
/// Every call of the turn lands in that one block: a block per call would
/// repeat the same header once per step.
fn activity(turn: &mut TurnView) -> &mut Activity {
    if turn.activity_mut().is_none() {
        turn.push(Entry::Activity(Activity::live()));
    }
    turn.activity_mut()
        .expect("the block was found above, or pushed there when absent")
}

/// The turn's answer, opened by the first text the model streams.
fn answer(turn: &mut TurnView) -> &mut Answer {
    if turn.answer_mut().is_none() {
        turn.push(Entry::Answer(Answer::default()));
    }
    turn.answer_mut()
        .expect("the answer was found above, or pushed there when absent")
}

/// Opens one running step in the turn's activity block.
fn requested(turn: &mut TurnView, tool: &str, arguments: &Value) {
    activity(turn).steps.push(Step {
        tool: tool.to_owned(),
        arguments: arguments.clone(),
        state: StepState::Running,
    });
}

/// Settles the run of the call that just finished.
fn completed(turn: &mut TurnView, tool: &str, summary: &str) {
    let state = if is_failure_summary(summary) {
        StepState::Failed {
            summary: summary.to_owned(),
        }
    } else {
        StepState::Done {
            summary: summary.to_owned(),
        }
    };
    let failed = matches!(state, StepState::Failed { .. });
    let activity = activity(turn);
    // A failed step keeps the block open for good: the reason has to stay
    // readable, so the whole turn's lines stay on screen.
    activity.failed |= failed;
    settle(activity, tool, state);
}

/// Settles the run of the call the operator refused.
fn denied(turn: &mut TurnView, tool: &str, reason: &str) {
    settle(
        activity(turn),
        tool,
        StepState::Denied {
            reason: reason.to_owned(),
        },
    );
}

/// Writes an outcome onto the oldest still-running step of `tool`, else opens
/// a step of its own.
///
/// Pairing is FIFO per tool name: the events carry no call id, so the oldest
/// outstanding call of that tool is the one that finished.
fn settle(activity: &mut Activity, tool: &str, state: StepState) {
    let running = activity
        .steps
        .iter()
        .position(|step| step.tool == tool && step.state.is_running());
    match running {
        Some(index) => activity.steps[index].state = state,
        None => activity.steps.push(Step {
            tool: tool.to_owned(),
            arguments: Value::Null,
            state,
        }),
    }
}

/// Ends the turn: a successful block folds to its one summary line.
///
/// A failed block never folds, and a turn with no tool call has no block to
/// fold.
fn end_turn(turn: &mut TurnView) {
    if let Some(activity) = turn.activity_mut()
        && !activity.failed
    {
        activity.folded = true;
    }
}
