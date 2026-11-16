//! Content-free outcomes for proposed calls refused before normal dispatch.
use crate::{AgentEventSink, ToolCall, ToolDefinition, ToolDiagnostic, ToolDiagnosticStatus};

pub(super) async fn refused(
    calls: &[ToolCall],
    definitions: &[ToolDefinition],
    sink: &dyn AgentEventSink,
) {
    for call in calls {
        for status in [
            ToolDiagnosticStatus::Requested,
            ToolDiagnosticStatus::Refused,
        ] {
            let mut diagnostic = ToolDiagnostic::special(&call.name, definitions, status);
            // The refusal happens before dispatch, so the transcript carries the
            // arguments the model sent and no result: nothing ran, and the row's
            // own `reason` names the refusal. Only the terminal row records it —
            // the requested row beside it is the same call's dispatch fact.
            if status == ToolDiagnosticStatus::Refused && !call.id.trim().is_empty() {
                diagnostic = diagnostic.with_transcript(crate::ToolCallTranscript::new(
                    &call.id,
                    &call.arguments.to_string(),
                    None,
                    0,
                ));
            }
            sink.observe_tool(diagnostic).await;
        }
    }
}
