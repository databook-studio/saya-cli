//! Feeding clarification-call results: the malformed-call error path, the
//! refusals for calls that did not run, and the gates the ask consults. Split
//! from [`super::clarification`] to keep each file by one concern.

use super::clarification_args::ClarificationError;
use super::{emit, tools};
use crate::{
    AgentEvent, AgentEventSink, AgentLimits, ChatMessage, ToolCall, ToolDefinition, ToolMetadata,
};

/// The tool result fed for a call that did not run because the turn paused on
/// the ask (siblings of a landed ask, and further asks).
pub(super) const PAUSED_REFUSAL: &str = "not executed: the turn paused on the clarification \
     question — the user will answer in their next message";

/// The tool result fed for a call that did not run because no ask landed.
pub(super) const NOT_EXECUTED_REFUSAL: &str = "not executed: the clarification did not land \
     this turn — retry it corrected and alone";

/// Feeds one malformed ask its validation error: the request surfaces, the
/// error rides back as the tool result, and the completion names the
/// validation failure — the same treatment the ordinary path gives an
/// invalid call. The caller has already checked cancellation.
pub(super) async fn feed_malformed(
    call: &ToolCall,
    error: &ClarificationError,
    definitions: &[ToolDefinition],
    limits: &AgentLimits,
    (events, sink): (&mut Vec<AgentEvent>, &dyn AgentEventSink),
    messages: &mut Vec<ChatMessage>,
    tool_metadata: &mut Vec<ToolMetadata>,
) {
    let effect = definitions
        .iter()
        .find(|definition| definition.name == call.name)
        .map(|definition| definition.effect);
    emit(
        events,
        sink,
        AgentEvent::tool_requested(call.name.clone(), call.arguments.clone(), effect),
    )
    .await;
    tool_metadata.push(ToolMetadata {
        name: call.name.clone(),
        status: "failed".into(),
        arguments: serde_json::to_string(&call.arguments).unwrap_or_default(),
        result_shape: None,
    });
    let (message, _) = tools::tool_message(
        call.id.clone(),
        serde_json::json!({"error": error.to_string()}),
        limits.context_byte_budget,
    );
    messages.push(message);
    emit(
        events,
        sink,
        AgentEvent::ToolCompleted {
            name: call.name.clone(),
            summary: "tool call failed validation".into(),
        },
    )
    .await;
}

/// Answers every call that did not run, without executing it: a provider
/// rejects a request whose assistant tool calls lack replies. `skip` holds
/// the call ids whose results were fed elsewhere.
pub(super) fn refuse_others(
    assistant: &ChatMessage,
    skip: &[&str],
    limits: &AgentLimits,
    messages: &mut Vec<ChatMessage>,
    reason: &str,
) {
    for call in &assistant.tool_calls {
        if skip.contains(&call.id.as_str()) {
            continue;
        }
        let (message, _) = tools::tool_message(
            call.id.clone(),
            serde_json::json!({"error": reason}),
            limits.context_byte_budget,
        );
        messages.push(message);
    }
}
