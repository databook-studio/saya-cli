use super::AgentError;
use crate::{ChatMessage, ToolCall, ToolDefinition, ToolExecutor};
use serde_json::Value;

pub(super) fn check_call(
    call: &ToolCall,
    definitions: &[ToolDefinition],
) -> Result<(), AgentError> {
    if call.name.is_empty()
        || !call.arguments.is_object()
        || !definitions.iter().any(|tool| tool.name == call.name)
    {
        Err(AgentError::InvalidToolCall)
    } else {
        Ok(())
    }
}
pub(super) async fn execute(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
) -> (Value, &'static str) {
    match tools.execute(name, arguments).await {
        Ok(value) => (value, "read-only database tool completed"),
        Err(_) => (
            serde_json::json!({"error":"read-only database tool failed"}),
            "read-only database tool failed",
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
