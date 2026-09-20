//! Boundary-event dispatch: every non-member event, after the group flush.

use super::Transcript;
use super::answer::apply_answer_event;
use super::group::apply_tool_edge;
use super::knowledge::apply_knowledge_event;
use super::thinking::push_reasoning_text;
use saya_agent::AgentEvent;

pub(crate) fn apply_boundary_event(
    transcript: &mut Transcript,
    event: AgentEvent,
    show_thinking: bool,
) {
    match event {
        AgentEvent::AssistantText { .. } | AgentEvent::TurnReset | AgentEvent::Complete => {
            let _ = apply_answer_event(transcript, event);
        }
        AgentEvent::ToolRequested { .. }
        | AgentEvent::ToolCompleted { .. }
        | AgentEvent::ToolDenied { .. } => {
            let _ = apply_tool_edge(transcript, event);
        }
        AgentEvent::KnowledgeSupplied { .. }
        | AgentEvent::KnowledgeOverridden { .. }
        | AgentEvent::KnowledgeLearningSkipped { .. }
        | AgentEvent::KnowledgeProposed { .. } => {
            let _ = apply_knowledge_event(transcript, event);
        }
        // The model's chain-of-thought. Shown only when the user opted in; otherwise
        // accepted and dropped, so the event never reaches the catch-all and never
        // renders as an error. When shown it lands as a dimmed `Thinking` block,
        // separate from the assistant answer and visually subordinate to it. Reasoning
        // is in-memory only: the transcript is never serialized, and the persisted
        // session types carry role + content only, so holding it here cannot reach a
        // session file regardless of the display toggle.
        AgentEvent::ReasoningText { text } => {
            push_reasoning_text(transcript, &text, show_thinking);
        }
        // The token counts one provider call reported. Accepted and dropped:
        // the transcript already shows a per-turn token line from the run
        // output at `Done`, and `/usage` breaks the session down, so a block
        // here would duplicate them. The event exists for the JSON/NDJSON
        // boundary; it must not reach the catch-all and disappear silently
        // into an error.
        AgentEvent::Usage { .. } => {}
        // Silent bookkeeping the grouper must still treat as a boundary.
        // `Usage` arrives after the answer finished streaming and carries no
        // content — but a group cannot span it, exactly as the piped adapter
        // flushes on every non-member event including silent ones.
        AgentEvent::KnowledgeLearningStarted => {}
        _ => {}
    }
}
