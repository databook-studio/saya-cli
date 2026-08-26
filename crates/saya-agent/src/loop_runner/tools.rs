use super::AgentError;
use crate::{ChatMessage, ToolCall, ToolDefinition, ToolExecutor};
use serde_json::Value;

/// Why a tool call cannot run as requested, or `None` when it is valid.
///
/// Recoverable problems (unknown tool, non-object arguments) are fed back to
/// the model as a tool result so it can correct itself; aborting the whole
/// run over one hallucinated name would waste the entire multi-turn effort.
/// A call with no id cannot anchor a well-formed tool message, so the caller
/// treats that case as fatal regardless of the reason returned here.
pub(super) fn invalid_reason(call: &ToolCall, definitions: &[ToolDefinition]) -> Option<String> {
    if !call.name.is_empty() && definitions.iter().any(|tool| tool.name == call.name) {
        if call.arguments.is_object() {
            return None;
        }
        return Some("arguments must be a JSON object".into());
    }
    let available = definitions
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "unknown tool '{}'; available tools: {available}",
        call.name
    ))
}
/// Runs a tool and returns its result with a completion summary that reflects
/// the tool's *declared* `read_only`, not its name. A write tool (`read_only:
/// false`, e.g. one that persists a candidate claim) must not read as a
/// "read-only" completion — that would be a false statement in the feature whose
/// pitch is that it does not overstate what it knows (spec P2d §4). The summary
/// drives `tool_metadata.status` via a `contains("failed")` check, so every
/// failure string keeps the substring "failed".
pub(super) async fn execute(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
    read_only: bool,
) -> (Value, &'static str) {
    match tools.execute(name, arguments).await {
        Ok(value) => (
            value,
            if read_only {
                "read-only database tool completed"
            } else {
                "local-state write completed"
            },
        ),
        Err(_) => (
            serde_json::json!({"error":"database tool failed"}),
            if read_only {
                "read-only database tool failed"
            } else {
                "local-state write failed"
            },
        ),
    }
}
pub(super) fn tool_message(id: String, result: Value) -> ChatMessage {
    ChatMessage {
        role: "tool".into(),
        content: bounded_json(&result),
        tool_calls: Vec::new(),
        tool_call_id: Some(id),
    }
}
fn bounded_json(value: &Value) -> String {
    let text = serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"error\":\"tool result unavailable\"}".into());
    if text.len() <= 65_536 {
        text
    } else {
        "{\"error\":\"tool result exceeded model context limit\"}".into()
    }
}
