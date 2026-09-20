//! Maps streamed agent events onto transcript blocks.
//!
//! Thin entry point: `apply_event` buffers group members and dispatches
//! everything else to [`apply_boundary_event`]. Per-event arms live in the
//! sibling concern modules; this file keeps only the entry-point glue.

mod answer;
mod boundary;
mod group;
mod knowledge;
mod thinking;

pub(crate) use super::transcript::{BlockKind, Transcript};
pub(crate) use boundary::apply_boundary_event;
pub(crate) use group::{buffer_tool_event, flush_tool_buffer, is_group_member, tool_call_detail};
use saya_agent::AgentEvent;

/// Applies one streamed agent event to the transcript.
///
/// `show_thinking` gates whether the model's chain-of-thought reaches the
/// transcript: off by default, so a user who did not ask for it never sees it.
/// When on, reasoning is pushed as a dimmed `Thinking` block — visually
/// subordinate to the answer, never mistakable for it. Either way reasoning is
/// in-memory only and never persisted.
///
/// Tool events buffer behind the shared grouper and flush at the next boundary
/// event: a run of tool calls lands as one collapsed block carrying the
/// Decision-2 summary (the same string the piped surface emits), expandable to
/// today's per-call `→` / `✓` lines. Streaming with the tail followed shows the
/// per-call lines as they arrive (a collapse imposed mid-stream would rewrite
/// history the user just watched); the group collapses when the boundary event
/// that ends it arrives.
pub(crate) fn apply_event(transcript: &mut Transcript, event: AgentEvent, show_thinking: bool) {
    // A caller that ends the stream after a tool run (tests, the panel's
    // final `Complete`) flushes on the boundary below. A caller that stops
    // mid-run with no boundary leaves buffered calls unrendered, so a trailing
    // flush would misattribute the next turn's text as this group's boundary —
    // keep the buffer, don't flush it here.
    if is_group_member(&event) {
        if let Some(other) = buffer_tool_event(transcript, event) {
            apply_boundary_event(transcript, other, show_thinking);
        }
        return;
    }
    // The retry discards the run in flight: the failure it reports belongs to
    // the transport, never to a collapsed summary.
    if matches!(event, AgentEvent::TurnReset) {
        transcript.discard_tool_buffer();
    } else {
        flush_tool_buffer(transcript);
    }
    apply_boundary_event(transcript, event, show_thinking);
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod answer_tests;
#[cfg(test)]
#[path = "group_collapse_tests.rs"]
mod group_collapse_tests;
#[cfg(test)]
#[path = "group_limits_tests.rs"]
mod group_limits_tests;
#[cfg(test)]
#[path = "group_live_tests.rs"]
mod group_live_tests;
#[cfg(test)]
#[path = "group_mark_tests.rs"]
mod group_mark_tests;
#[cfg(test)]
#[path = "knowledge_override_tests.rs"]
mod knowledge_override_tests;
#[cfg(test)]
#[path = "knowledge_supplied_tests.rs"]
mod knowledge_supplied_tests;
#[cfg(test)]
#[path = "reasoning_tests.rs"]
mod reasoning_tests;
