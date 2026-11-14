//! Recoverable validation replies, before admission.
use super::super::{check_cancelled, emit, output, tool_outcome::ToolOutcome, tools};
use super::state::{TurnOutputs, TurnState};
use crate::{AgentError, AgentEvent, ToolCall, ToolDiagnostic, ToolDiagnosticStatus};

pub(super) async fn handle(
    state: &TurnState<'_>,
    outputs: &mut TurnOutputs<'_>,
    call: &ToolCall,
    cap: usize,
) -> Result<bool, AgentError> {
    let definitions = state.definitions;
    let sink = state.sink;
    let cancellation = state.cancellation;
    let events = &mut *outputs.events;
    let messages = &mut *outputs.messages;
    let tool_metadata = &mut *outputs.tool_metadata;
    check_cancelled(cancellation)?;
    let diagnostic_definition = definitions
        .iter()
        .find(|definition| definition.name == call.name);
    sink.observe_tool(ToolDiagnostic::new(
        diagnostic_definition,
        ToolDiagnosticStatus::Requested,
    ))
    .await;
    if let Some(reason) = tools::invalid_reason(call, definitions) {
        if call.id.trim().is_empty() {
            sink.observe_tool(
                ToolDiagnostic::new(diagnostic_definition, ToolDiagnosticStatus::Failed)
                    .with_failure_detail(&call.arguments.to_string(), || reason.clone()),
            )
            .await;
            return Err(AgentError::InvalidToolCall);
        }
        check_cancelled(cancellation)?;
        // A call to an unknown tool has no declared effect to carry —
        // `None`, never a fabricated one; the renderer claims nothing it
        // cannot prove for a tool that does not exist.
        let effect = definitions
            .iter()
            .find(|definition| definition.name == call.name)
            .map(|definition| definition.effect);
        emit(
            events,
            sink,
            AgentEvent::ToolRequested {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                effect,
            },
        )
        .await;
        // Feed the problem back as the tool result so the model can
        // retry with a valid call on its next turn.
        tool_metadata.push(crate::ToolMetadata {
            name: call.name.clone(),
            status: "failed".into(),
            arguments: serde_json::to_string(&call.arguments).unwrap_or_default(),
            result_shape: None,
        });
        let completed = tools::complete_tool(
            call.id.clone(),
            ToolOutcome::refused(
                serde_json::json!({"error": reason}),
                "tool call failed validation".into(),
            ),
            false,
            cap,
        );
        // The refusal is fed back as the tool result, so the recorded reason is
        // the exact text the model reads for this call.
        let arguments_json = call.arguments.to_string();
        let result_json = serde_json::to_string(&completed.result).unwrap_or_default();
        sink.observe_tool(
            ToolDiagnostic::new(diagnostic_definition, ToolDiagnosticStatus::Failed)
                .with_failure_detail(&arguments_json, || {
                    ToolDiagnostic::result_error_text(&completed.result)
                })
                .with_transcript(crate::ToolCallTranscript::new(
                    &call.id,
                    &arguments_json,
                    Some(&result_json),
                    0,
                )),
        )
        .await;
        messages.push(completed.message);
        emit(
            events,
            sink,
            AgentEvent::ToolCompleted {
                name: call.name.clone(),
                summary: output::completion_summary(&completed.summary, completed.truncated),
            },
        )
        .await;
        return Ok(true);
    }
    // A call that failed earlier in this run is executed again: repeating a
    // call is how the model learns it is hopeless, and the failure memory
    // counts the repeats to offer advice rather than to block the call.
    Ok(false)
}
