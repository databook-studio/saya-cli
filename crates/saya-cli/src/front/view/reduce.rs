//! The reducer: one session event folded into the transcript view.
//!
//! Pure — no I/O, no clock, no terminal — so a turn's whole layout is a
//! function of the events it received, and a test can fold a recorded stream
//! without a TUI.

use saya_agent::AgentEvent;
use serde_json::Value;

use super::entry::{Activity, Answer, Entry, Notice, Step, StepState, TranscriptView, TurnView};
use super::entry_blocks::{Clarification, Error, Helper, HelperState, Stop};
use super::entry_result::ResultPreview;
use crate::chat::{ChatEvent, ChatOutcome};
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
        ChatEvent::TurnEnded(outcome) => end_turn(view.current(), outcome),
        // A prepared run and a call put to the operator are read by the status
        // line and the approval card, not by the transcript.
        ChatEvent::Ready { .. } | ChatEvent::Question(_) => {}
    }
}

/// Folds one event of the engine's own loop.
fn apply_agent(turn: &mut TurnView, event: &AgentEvent) {
    match event {
        AgentEvent::ToolRequested {
            name,
            arguments,
            caller,
            ..
        } => match caller {
            Some(caller) => helper_requested(turn, caller),
            None => requested(turn, name, arguments),
        },
        AgentEvent::ToolCompleted {
            name,
            summary,
            caller,
        } => match caller {
            Some(_) => {}
            None => completed(turn, name, summary),
        },
        AgentEvent::ToolDenied {
            name,
            reason,
            caller,
            ..
        } => match caller {
            Some(_) => {}
            None => denied(turn, name, reason),
        },
        AgentEvent::AssistantText { text } => answer(turn).text.push_str(text),
        // The model stopped to ask instead of assuming: the question is a block
        // of the turn, and the operator's answer is the next turn (`02` §3.9).
        AgentEvent::ClarificationNeeded { question, options } => {
            turn.push(Entry::Question(Clarification {
                question: question.clone(),
                options: options.clone(),
            }))
        }
        // The attempt that produced the text so far was discarded, and the
        // retried stream re-emits the whole answer as fresh deltas.
        AgentEvent::TurnReset => {
            if let Some(answer) = turn.answer_mut() {
                answer.text.clear();
            }
        }
        // What recall supplied this turn: the same shaper the text front
        // prints, so a line front and the TUI say the same words. An event that
        // shapes to nothing (recall ran and found nothing, nothing dropped)
        // adds no line.
        AgentEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        } => {
            let text =
                crate::render::knowledge_supplied_text(*outcome, contracts, *dropped_by_bounds);
            let text = text.trim_end();
            if !text.is_empty() {
                turn.push(Entry::Notice(Notice {
                    message: text.to_owned(),
                }));
            }
        }
        // Metrics, recovery and the remaining knowledge events are progress and
        // evidence; the status line and the inspectors read them, not the
        // turn's blocks.
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

/// One helper's work, opened by the first call the helper's own loop made.
///
/// A helper's calls fold into one block of their own, keyed by its caller id —
/// never into the root's activity group, which is the root's own work (`02`
/// §3.6). The block's brief stays empty: an event carries the caller and the
/// call, not the assignment the helper was given.
fn helper_requested(turn: &mut TurnView, caller: &str) {
    match turn.helper_mut(caller) {
        Some(helper) => helper.steps += 1,
        None => turn.push(Entry::Helper(Helper {
            id: caller.to_owned(),
            question: String::new(),
            steps: 1,
            state: HelperState::Running,
        })),
    }
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
    // The call that launched the helpers returned, so the helpers it launched
    // have ended: their own blocks settle here, because the forwarded stream
    // carries their calls and never their completion.
    if tool == "helpers" {
        turn.settle_helpers();
    }
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

/// Ends the turn: a successful block folds to its one summary line, and, when
/// the turn stopped on something the operator has to read, its stop or its
/// error becomes a block of its own.
///
/// A failed block never folds, and a turn with no tool call has no block to
/// fold.
fn end_turn(turn: &mut TurnView, outcome: &ChatOutcome) {
    if let Some(activity) = turn.activity_mut()
        && !activity.failed
    {
        activity.folded = true;
    }
    if let Some(entry) = ended_entry(outcome) {
        turn.push(entry);
    }
}

/// The block an ending leaves behind, when it left one: a stop with the
/// dimension that ran out or the guard's own sentence, an error with its
/// reason, or the operator's cancel. A turn the model answered leaves none.
fn ended_entry(outcome: &ChatOutcome) -> Option<Entry> {
    match outcome {
        ChatOutcome::Answered => None,
        ChatOutcome::Stopped(budget) => Some(Entry::Stop(Stop::Budget {
            dimension: budget.dimension,
            // The key and value that bound the turn are the surface's facts:
            // the outcome names the dimension, not the configuration it came
            // from.
            limit: None,
        })),
        ChatOutcome::Paused { message } => Some(Entry::Stop(Stop::Paused {
            message: message.clone(),
        })),
        ChatOutcome::Failed { message } => Some(Entry::Error(Error {
            message: message.clone(),
        })),
        ChatOutcome::Cancelled => Some(Entry::Stop(Stop::Cancelled)),
    }
}
