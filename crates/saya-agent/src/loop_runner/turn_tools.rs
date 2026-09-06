//! Executes the tool calls of one assistant turn — concurrently when every
//! call is valid and auto-runnable, otherwise sequentially with approval
//! gating. The caller trims the intra-loop context budget after either path
//! returns; this function no longer signals which path ran, because both
//! paths feed the same trim.

use super::{check_cancelled, emit, failed_statements, output, tool_record, tools};
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
    failed: &mut failed_statements::FailedStatements,
    last_successful_sql: &mut Option<String>,
) -> Result<(), AgentError> {
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
                && !failed_statements::is_repeat(failed, call)
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
            failed_statements::record_outcome(
                failed,
                last_successful_sql,
                failed_statements::sql_of(call),
                &result,
                true,
                summary,
            );
            // Read the value-free shape before `tool_message` takes ownership
            // of `result`: only `row_count` and `columns` are read, so no cell
            // value is copied.
            let result_shape = tool_record::result_shape_of(&result);
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
                arguments: serde_json::to_string(&call.arguments).unwrap_or_default(),
                result_shape,
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
        return Ok(());
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
                arguments: serde_json::to_string(&call.arguments).unwrap_or_default(),
                result_shape: None,
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
        // A byte-identical repeat of a statement that already failed in this
        // run is refused rather than re-executed: the failure is deterministic
        // at parse/safety time, so re-running it wastes the turn (the benchmark
        // saw one statement re-sent 384 times). Feed the prior error back as
        // the tool result so the model has the information it needs to change
        // approach. This does not fail the turn — the point is to return
        // signal cheaply, not to abort.
        if let Some(sql) = failed_statements::sql_of(&call)
            && let Some(prior) = failed.prior_error(sql)
        {
            check_cancelled(cancellation)?;
            let (result, summary) = failed_statements::refuse_repeat(prior);
            tool_metadata.push(crate::ToolMetadata {
                name: call.name.clone(),
                status: "failed".into(),
                arguments: serde_json::to_string(&call.arguments).unwrap_or_default(),
                result_shape: None,
            });
            let (message, _) = tools::tool_message(call.id, result, limits.context_byte_budget);
            messages.push(message);
            emit(
                events,
                sink,
                AgentEvent::ToolCompleted {
                    name: call.name,
                    summary: summary.into(),
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
        // Capture the SQL before `execute` moves `call.arguments`; only SQL
        // statements are tracked for repeat refusal and salvage nomination.
        let sql = failed_statements::sql_of(&call).map(str::to_owned);
        // Capture the serialized arguments before `execute` moves
        // `call.arguments` — the persisted record carries what the model sent
        // (the statement for a SQL tool), and the value-free result shape is
        // read from `result` after the call resolves. Cell values never reach
        // either; `result_shape_of` reads only `row_count` and `columns`.
        let arguments_json = serde_json::to_string(&call.arguments).unwrap_or_default();
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
        failed_statements::record_outcome(
            failed,
            last_successful_sql,
            sql.as_deref(),
            &result,
            executed,
            summary,
        );
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
            arguments: arguments_json,
            result_shape: tool_record::result_shape_of(&result),
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
    Ok(())
}
