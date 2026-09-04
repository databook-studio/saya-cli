//! Executes the tool calls of one assistant turn — concurrently when every
//! call is valid and auto-runnable, otherwise sequentially with approval
//! gating. Returns `true` when the batch path ran (the caller continues the
//! loop) or `false` when the sequential path ran (the caller proceeds to the
//! intra-loop context trim).

use super::{check_cancelled, emit, output, tools};
use crate::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, ApprovalDecider, CancellationToken,
    ChatMessage, ToolDefinition, ToolExecutor,
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_turn_tools(
    tools: &dyn ToolExecutor,
    assistant: ChatMessage,
    definitions: &[ToolDefinition],
    limits: &AgentLimits,
    approval: &dyn ApprovalDecider,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    messages: &mut Vec<ChatMessage>,
    used_bounded_sql_query: &mut bool,
    tool_metadata: &mut Vec<crate::ToolMetadata>,
) -> Result<bool, AgentError> {
    // When every call in the message is valid and auto-runnable, the
    // calls are independent: run them concurrently instead of paying
    // their latency sequentially. `auto_runnable` is the single policy
    // for "may this run with no questions asked"; the sequential path
    // below applies the same gates, so the two cannot drift.
    let batch_parallel = assistant.tool_calls.len() > 1
        && assistant.tool_calls.iter().all(|call| {
            tools::invalid_reason(call, definitions).is_none()
                && definitions
                    .iter()
                    .find(|definition| definition.name == call.name)
                    .is_some_and(|definition| tools::auto_runnable(definition, limits))
        });
    if batch_parallel {
        check_cancelled(cancellation)?;
        for call in &assistant.tool_calls {
            emit(
                events,
                sink,
                AgentEvent::ToolRequested {
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            )
            .await;
            if definitions
                .iter()
                .find(|definition| definition.name == call.name)
                .is_some_and(|definition| definition.effect.database_data)
            {
                *used_bounded_sql_query = true;
            }
        }
        let results = tools::execute_batch(tools, &assistant.tool_calls.clone(), definitions).await;
        for (call, (result, summary)) in assistant.tool_calls.iter().zip(results) {
            let (message, truncated) =
                tools::tool_message(call.id.clone(), result, limits.context_byte_budget);
            tool_metadata.push(crate::ToolMetadata {
                name: call.name.clone(),
                status: if summary.contains("failed") {
                    "failed"
                } else {
                    "completed"
                }
                .into(),
            });
            messages.push(message);
            check_cancelled(cancellation)?;
            emit(
                events,
                sink,
                AgentEvent::ToolCompleted {
                    name: call.name.clone(),
                    summary: output::completion_summary(summary, truncated),
                },
            )
            .await;
        }
        return Ok(true);
    }
    for call in assistant.tool_calls {
        if let Some(reason) = tools::invalid_reason(&call, definitions) {
            if call.id.trim().is_empty() {
                return Err(AgentError::InvalidToolCall);
            }
            check_cancelled(cancellation)?;
            emit(
                events,
                sink,
                AgentEvent::ToolRequested {
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            )
            .await;
            // Feed the problem back as the tool result so the model can
            // retry with a valid call on its next turn.
            tool_metadata.push(crate::ToolMetadata {
                name: call.name.clone(),
                status: "failed".into(),
            });
            let (message, _) = tools::tool_message(
                call.id,
                serde_json::json!({"error": reason}),
                limits.context_byte_budget,
            );
            messages.push(message);
            emit(
                events,
                sink,
                AgentEvent::ToolCompleted {
                    name: call.name,
                    summary: "tool call failed validation".into(),
                },
            )
            .await;
            continue;
        }
        check_cancelled(cancellation)?;
        emit(
            events,
            sink,
            AgentEvent::ToolRequested {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            },
        )
        .await;
        let definition = definitions
            .iter()
            .find(|tool| tool.name == call.name)
            .expect("validated");
        let approved = !definition.effect.requires_approval
            || approval.approve(definition, &call.arguments).await;
        // Apply the same policy the batch path consults (`auto_runnable`),
        // split into its gates so the denial can name which one refused.
        // `requires_approval` was already resolved into `approved`, so a
        // tool that needed approval and got it still runs; the remaining
        // gates bind whether or not approval was granted. This is the one
        // place the sequential path decides auto-run — keeping it here in
        // terms of the shared gates means a gate added to `tools.rs`
        // cannot apply to the batch path and not this one.
        let candidate_denied = tools::candidate_denied(definition, limits);
        let side_effect_denied = tools::external_side_effect_gated(definition);
        let executed = approved && !candidate_denied && !side_effect_denied;
        let (result, summary) = if executed {
            check_cancelled(cancellation)?;
            // Indicates a database-row-producing query tool ran.
            if definition.effect.database_data {
                *used_bounded_sql_query = true;
            }
            tools::execute(tools, &call.name, call.arguments, definition.read_only).await
        } else {
            emit(
                events,
                sink,
                AgentEvent::ToolDenied {
                    name: call.name.clone(),
                    reason: if side_effect_denied {
                        "external side effect requires approval".into()
                    } else if candidate_denied {
                        "candidate writes are not permitted".into()
                    } else {
                        "approval was not granted".into()
                    },
                },
            )
            .await;
            (
                serde_json::json!({"error":"tool call denied by approval policy"}),
                "read-only database tool denied",
            )
        };
        tool_metadata.push(crate::ToolMetadata {
            name: call.name.clone(),
            status: if executed {
                if summary.contains("failed") {
                    "failed"
                } else {
                    "completed"
                }
            } else {
                "denied"
            }
            .into(),
        });
        let (message, truncated) = tools::tool_message(call.id, result, limits.context_byte_budget);
        messages.push(message);
        if executed {
            check_cancelled(cancellation)?;
            emit(
                events,
                sink,
                AgentEvent::ToolCompleted {
                    name: call.name,
                    summary: output::completion_summary(summary, truncated),
                },
            )
            .await;
        }
    }
    Ok(false)
}
