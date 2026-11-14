#[cfg(test)]
use crate::CancellationToken;
use crate::{ToolDefinition, ToolExecutionContext, ToolExecutor};
use serde_json::Value;

use super::super::tool_outcome::ToolOutcome;
#[cfg(test)]
use super::MAX_TOOL_MESSAGE_BYTES;
use super::completions::completion_summaries;
use super::failure_summary::failure_summary_with_reason;

/// Ceiling on how many tool calls from one assistant message run at once.
/// This scheduler bound is independent of connector or profile pool settings:
/// calls may target separate connections, so it is not a claim about one
/// shared database pool.
pub(in crate::loop_runner) const MAX_CONCURRENT_TOOL_CALLS: usize = 4;

/// Runs a tool and returns its result with a completion summary that reflects
/// the tool's *declaration*, not its name (see [`completion_summaries`]). The
/// The typed outcome returned beside the summary drives turn-tool status and
/// failure memory; summary text remains model-facing only.
#[cfg(test)]
pub(in crate::loop_runner) async fn execute(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
    definition: Option<&ToolDefinition>,
) -> (Value, String) {
    execute_with_context(
        tools,
        name,
        arguments,
        definition,
        ToolExecutionContext {
            result_cap: MAX_TOOL_MESSAGE_BYTES,
            cancellation: CancellationToken::new(),
        },
    )
    .await
}

#[cfg(test)]
async fn execute_with_context(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
    definition: Option<&ToolDefinition>,
    context: ToolExecutionContext,
) -> (Value, String) {
    let outcome = execute_with_outcome(tools, name, arguments, definition, context).await;
    (outcome.result, outcome.summary)
}

pub(in crate::loop_runner) async fn execute_with_outcome(
    tools: &dyn ToolExecutor,
    name: &str,
    arguments: Value,
    definition: Option<&ToolDefinition>,
    context: ToolExecutionContext,
) -> ToolOutcome {
    match tools
        .execute_with_context(name, arguments.clone(), context)
        .await
    {
        Ok(value) => {
            let (completed, _) = completion_summaries(definition, name, &arguments, Some(&value));
            ToolOutcome::completed(name, &arguments, value, completed)
        }
        // The reason reaches the model so it can adjust (e.g. a
        // safety-layer rejection naming what is not allowed); the bounded,
        // redacted form reaches the human summary so a refusal reads
        // differently from a runtime failure. The fourth value records
        // whether the executor proved the failure changed nothing: the
        // result JSON stays the plain error text either way.
        Err(error) => {
            let (_, failed) = completion_summaries(definition, name, &arguments, None);
            let summary = failure_summary_with_reason(&failed, &error.to_string());
            ToolOutcome::failed(name, &arguments, &error, summary)
        }
    }
}
