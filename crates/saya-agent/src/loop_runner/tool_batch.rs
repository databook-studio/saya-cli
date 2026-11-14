use crate::{
    AgentEventSink, AgentMetric, AgentMetricUseful, CancellationToken, ChatMessage, ToolCall,
    ToolDefinition, ToolDiagnostic, ToolDiagnosticStatus, ToolExecutionContext, ToolExecutor,
};

use serde_json::Value;

use super::{failed_statements, tool_outcome::ToolOutcome, tool_record, tool_shape, tools};

const MAX_COMPLETION_SUMMARY_BYTES: usize = 1024;

#[cfg(test)]
#[path = "tool_batch_tests.rs"]
mod tests;

pub(super) struct CompletedTool {
    pub(super) message: ChatMessage,
    pub(super) summary: String,
    pub(super) succeeded: bool,
    pub(super) executed: bool,
    /// The executor proved this failed call changed nothing; only a
    /// settlement mapping may treat it as a known outcome.
    pub(super) effect_free: bool,
    pub(super) truncated: bool,
    pub(super) result_shape: Option<crate::ToolResultShape>,
    pub(super) observation: super::coding_checkpoint::Observation,
    /// The exact value the executor returned, kept so the repeat note can be
    /// appended to the failed result the model reads and the text re-shaped
    /// around it.
    pub(super) result: Value,
}

impl CompletedTool {
    /// Appends the repeat note to this failed call's result and re-shapes the
    /// model-facing text, so the note is part of the result the model reads
    /// rather than a decoration beside it.
    pub(super) fn append_note(&mut self, note: &str, cap: usize) {
        failed_statements::append_note(&mut self.result, note);
        let shaped = tool_shape::shape_tool_result_at_cap(&self.result, cap);
        self.message.content = shaped.text;
        self.truncated = shaped.truncated;
    }
}

pub(super) async fn execute_batch(
    executor: &dyn ToolExecutor,
    calls: &[ToolCall],
    definitions: &[ToolDefinition],
    result_caps: &[usize],
    cancellation: &CancellationToken,
    sink: &dyn AgentEventSink,
) -> Vec<CompletedTool> {
    use futures_util::{StreamExt, stream};

    // Materialize the bounded list so the spawned loop retains a Send future;
    // a lazy borrowed async iterator introduces a higher-ranked closure lifetime.
    let pending = calls
        .iter()
        .zip(result_caps)
        .map(|(call, &cap)| {
            let definition = definitions
                .iter()
                .find(|definition| definition.name == call.name);
            let name = call.name.clone();
            let arguments = call.arguments.clone();
            // The arguments the model sent, serialized before `execute` takes
            // them, so the recorded transcript carries what was asked for.
            let arguments_json = serde_json::to_string(&arguments).unwrap_or_default();
            let context = ToolExecutionContext {
                result_cap: cap,
                cancellation: cancellation.clone(),
            };
            async move {
                let executed = !cancellation.is_cancelled();
                let started = std::time::Instant::now();
                let outcome = if !executed {
                    ToolOutcome::refused(
                        serde_json::json!({"error":"tool was not executed after cancellation"}),
                        "tool not executed after cancellation".to_owned(),
                    )
                } else {
                    let outcome = tools::execute_with_outcome(
                        executor, &name, arguments, definition, context,
                    )
                    .await;
                    if outcome.succeeded {
                        sink.metric(AgentMetric::Useful(AgentMetricUseful::SuccessfulTool))
                            .await;
                    }
                    outcome
                };
                let status = if executed {
                    ToolDiagnostic::result_status(&outcome.result, outcome.succeeded)
                } else {
                    ToolDiagnosticStatus::Refused
                };
                let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let result_json = serde_json::to_string(&outcome.result).unwrap_or_default();
                sink.observe_tool(ToolDiagnostic::new(definition, status).with_transcript(
                    crate::ToolCallTranscript::new(
                        &call.id,
                        &arguments_json,
                        Some(&result_json),
                        duration_ms,
                    ),
                ))
                .await;
                complete_tool(call.id.clone(), outcome, executed, cap)
            }
        })
        .collect::<Vec<_>>();
    stream::iter(pending)
        .buffered(tools::MAX_CONCURRENT_TOOL_CALLS)
        .collect()
        .await
}

pub(super) fn complete_tool(
    id: String,
    outcome: ToolOutcome,
    executed: bool,
    cap: usize,
) -> CompletedTool {
    let ToolOutcome {
        result,
        summary,
        succeeded,
        effect_free,
        observation,
        images,
    } = outcome;
    let result_shape = tool_record::result_shape_of(&result);
    let shaped = tool_shape::shape_tool_result_at_cap(&result, cap);
    CompletedTool {
        message: ChatMessage {
            role: "tool".into(),
            content: shaped.text,
            tool_calls: Vec::new(),
            tool_call_id: Some(id),
            images,
            reasoning: None,
            reasoning_signature: None,
        },
        summary: bound_summary(&summary),
        succeeded,
        executed,
        effect_free,
        truncated: shaped.truncated,
        result_shape,
        observation,
        result,
    }
}

/// Every call in a turn gets the same per-call cap. There is no per-turn
/// pool: one call's result is never sized by how many calls share the batch.
pub(super) fn turn_result_caps(context_byte_budget: usize, calls: usize) -> Vec<usize> {
    vec![tool_shape::tool_message_cap(context_byte_budget); calls]
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
