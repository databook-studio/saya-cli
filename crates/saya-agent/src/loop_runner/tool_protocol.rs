//! Whole-message tool-call protocol checks that must precede every side action.

use crate::{AgentEventSink, ToolCall, ToolDefinition, ToolDiagnostic, ToolDiagnosticStatus};
use std::collections::HashSet;

/// Collection bounds the calls and bytes while the provider response is built.
/// Once complete, every caller validates these reply IDs before treating the
/// response as an assistant message. This set keeps borrowed IDs only.
pub(super) async fn valid_ids(
    calls: &[ToolCall],
    definitions: &[ToolDefinition],
    sink: &dyn AgentEventSink,
) -> bool {
    let mut ids = HashSet::with_capacity(calls.len());
    let valid = calls
        .iter()
        .map(|call| !call.id.trim().is_empty() && ids.insert(call.id.as_str()))
        .collect::<Vec<_>>();
    if valid.iter().all(|valid| *valid) {
        return true;
    }
    // Invalid reply IDs refuse the entire message before any side action.
    for (call, valid) in calls.iter().zip(valid) {
        let terminal = if valid {
            ToolDiagnosticStatus::Refused
        } else {
            ToolDiagnosticStatus::Failed
        };
        for status in [ToolDiagnosticStatus::Requested, terminal] {
            sink.observe_tool(ToolDiagnostic::special(&call.name, definitions, status))
                .await;
        }
    }
    false
}
