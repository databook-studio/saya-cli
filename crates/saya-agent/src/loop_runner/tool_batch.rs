use crate::{ChatMessage, ToolCall, ToolDefinition, ToolExecutor};
use serde_json::Value;

use super::{failed_statements, tool_record, tool_shape, tools};

const MAX_TURN_TOOL_RESULT_BYTES: usize = 256 * 1024;
const MAX_COMPLETION_SUMMARY_BYTES: usize = 1024;

pub(super) struct CompletedTool {
    pub(super) message: ChatMessage,
    pub(super) summary: String,
    pub(super) succeeded: bool,
    pub(super) truncated: bool,
    pub(super) result_shape: Option<crate::ToolResultShape>,
    pub(super) failure_reason: Option<String>,
}

pub(super) async fn execute_batch(
    executor: &dyn ToolExecutor,
    calls: &[ToolCall],
    definitions: &[ToolDefinition],
    result_caps: &[usize],
) -> Vec<CompletedTool> {
    use futures_util::{StreamExt, stream};

    let pending = calls.iter().zip(result_caps).map(|(call, &cap)| {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == call.name);
        let name = call.name.clone();
        let arguments = call.arguments.clone();
        async move {
            let (result, summary, succeeded) =
                tools::execute_with_outcome(executor, &name, arguments, definition, cap).await;
            complete_tool(call.id.clone(), result, summary, succeeded, cap)
        }
    });
    stream::iter(pending)
        .buffered(tools::MAX_CONCURRENT_TOOL_CALLS)
        .collect()
        .await
}

pub(super) fn complete_tool(
    id: String,
    result: Value,
    summary: String,
    succeeded: bool,
    cap: usize,
) -> CompletedTool {
    let result_shape = tool_record::result_shape_of(&result);
    let failure_reason = (!succeeded).then(|| {
        failed_statements::bound_error(
            result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
    });
    let shaped = tool_shape::shape_tool_result_at_cap(&result, cap);
    CompletedTool {
        message: ChatMessage {
            role: "tool".into(),
            content: shaped.text,
            tool_calls: Vec::new(),
            tool_call_id: Some(id),
        },
        summary: bound_summary(&summary),
        succeeded,
        truncated: shaped.truncated,
        result_shape,
        failure_reason,
    }
}

pub(super) fn turn_result_caps(context_byte_budget: usize, calls: usize) -> Vec<usize> {
    if calls == 0 {
        return Vec::new();
    }
    let total = context_byte_budget.min(MAX_TURN_TOOL_RESULT_BYTES);
    let each = total / calls;
    let remainder = total % calls;
    (0..calls)
        .map(|index| {
            each.saturating_add(usize::from(index < remainder))
                .min(tool_shape::MAX_TOOL_MESSAGE_BYTES)
        })
        .collect()
}

fn bound_summary(summary: &str) -> String {
    if summary.len() <= MAX_COMPLETION_SUMMARY_BYTES {
        return summary.to_owned();
    }
    let marker = "...[truncated]";
    let head = tools::floor_boundary(
        summary,
        MAX_COMPLETION_SUMMARY_BYTES.saturating_sub(marker.len()),
    );
    format!("{}{marker}", &summary[..head])
}
