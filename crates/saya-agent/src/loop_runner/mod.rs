mod output;
mod receive;
mod tools;

use crate::{
    AgentEvent, AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken, ChatProvider,
    TokenUsage, ToolDefinition, ToolExecutor,
};

pub use output::{AgentError, AgentLimits, AgentOutput};

#[allow(clippy::too_many_arguments)]
pub async fn run_agent_with_sink(
    provider: &dyn ChatProvider,
    tools: &dyn ToolExecutor,
    request: AgentRequest,
    definitions: Vec<ToolDefinition>,
    limits: AgentLimits,
    approval: &dyn ApprovalDecider,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
) -> Result<AgentOutput, AgentError> {
    let mut messages = crate::history::build_messages(
        request.system_prompt.as_deref(),
        &request.context_blocks,
        &request.prompt,
        &request.history,
    )?;
    let mut events = Vec::new();
    let mut tool_count = 0;
    let mut used_bounded_sql_query = false;
    let mut tool_metadata = Vec::new();
    let mut usage = TokenUsage::default();
    for _ in 0..limits.max_turns {
        check_cancelled(&cancellation)?;
        let (assistant, turn_usage, _reasoning) = receive::receive(
            provider,
            &request.model,
            &messages,
            &definitions,
            sink,
            &cancellation,
            &mut events,
        )
        .await?;
        // Providers report cumulative counts per response; sum across turns.
        usage.input_tokens += turn_usage.input_tokens;
        usage.output_tokens += turn_usage.output_tokens;
        // `_reasoning` is the turn's captured chain-of-thought. It is held
        // turn-local and dropped here: surfacing it to the user is S23b, which
        // this slice does not start. Crucially it is NOT pushed onto
        // `messages` — `assistant` (a `ChatMessage`) is what gets replayed to
        // the provider as history, and `ChatMessage` has no reasoning field, so
        // reasoning cannot leak into the next turn's request (S23 invariant 2,
        // structural in the type choice).
        messages.push(assistant.clone());
        if assistant.tool_calls.is_empty() {
            check_cancelled(&cancellation)?;
            emit(&mut events, sink, AgentEvent::Complete).await;
            return Ok(AgentOutput {
                answer: assistant.content,
                events,
                used_bounded_sql_query,
                tool_metadata,
                usage,
            });
        }
        // When every call in the message is valid and auto-runnable, the
        // calls are independent: run them concurrently instead of paying
        // their latency sequentially. `auto_runnable` is the single policy
        // for "may this run with no questions asked"; the sequential path
        // below applies the same gates, so the two cannot drift.
        let batch_parallel = assistant.tool_calls.len() > 1
            && assistant.tool_calls.iter().all(|call| {
                tools::invalid_reason(call, &definitions).is_none()
                    && definitions
                        .iter()
                        .find(|definition| definition.name == call.name)
                        .is_some_and(|definition| tools::auto_runnable(definition, &limits))
            });
        if batch_parallel {
            tool_count += assistant.tool_calls.len();
            if tool_count > limits.max_tool_calls {
                return Err(AgentError::Limit("tool calls"));
            }
            check_cancelled(&cancellation)?;
            for call in &assistant.tool_calls {
                emit(
                    &mut events,
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
                    used_bounded_sql_query = true;
                }
            }
            let results =
                tools::execute_batch(tools, &assistant.tool_calls.clone(), &definitions).await;
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
                check_cancelled(&cancellation)?;
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolCompleted {
                        name: call.name.clone(),
                        summary: output::completion_summary(summary, truncated),
                    },
                )
                .await;
            }
            continue;
        }
        for call in assistant.tool_calls {
            if let Some(reason) = tools::invalid_reason(&call, &definitions) {
                if call.id.trim().is_empty() {
                    return Err(AgentError::InvalidToolCall);
                }
                tool_count += 1;
                if tool_count > limits.max_tool_calls {
                    return Err(AgentError::Limit("tool calls"));
                }
                check_cancelled(&cancellation)?;
                emit(
                    &mut events,
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
                    &mut events,
                    sink,
                    AgentEvent::ToolCompleted {
                        name: call.name,
                        summary: "tool call failed validation".into(),
                    },
                )
                .await;
                continue;
            }
            tool_count += 1;
            if tool_count > limits.max_tool_calls {
                return Err(AgentError::Limit("tool calls"));
            }
            check_cancelled(&cancellation)?;
            emit(
                &mut events,
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
            let candidate_denied = tools::candidate_denied(definition, &limits);
            let side_effect_denied = tools::external_side_effect_gated(definition);
            let executed = approved && !candidate_denied && !side_effect_denied;
            let (result, summary) = if executed {
                check_cancelled(&cancellation)?;
                // Indicates a database-row-producing query tool ran.
                if definition.effect.database_data {
                    used_bounded_sql_query = true;
                }
                tools::execute(tools, &call.name, call.arguments, definition.read_only).await
            } else {
                emit(
                    &mut events,
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
            let (message, truncated) =
                tools::tool_message(call.id, result, limits.context_byte_budget);
            messages.push(message);
            if executed {
                check_cancelled(&cancellation)?;
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolCompleted {
                        name: call.name,
                        summary: output::completion_summary(summary, truncated),
                    },
                )
                .await;
            }
        }
        // Intra-loop context budget: the pre-loop trim bounds history, but
        // assistant turns and tool results accumulate here. Trim the oldest
        // tool-result pairs (the assistant turn that issued each call plus its
        // `tool` message) until the conversation fits, keeping the newest
        // context — the same recency policy the pre-loop path uses. A single
        // result is already capped at construction, so this resolves
        // accumulation; if trimming everything still leaves the newest result
        // over budget, truncate it rather than aborting the whole run (S4
        // invariant 1: one result must never kill the run by itself).
        output::trim_to_budget(&mut messages, limits.context_byte_budget);
    }
    Err(AgentError::Limit("turns"))
}

pub(super) async fn emit(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    event: AgentEvent,
) {
    sink.emit(event.clone()).await;
    events.push(event);
}
fn check_cancelled(token: &CancellationToken) -> Result<(), AgentError> {
    if token.is_cancelled() {
        Err(AgentError::Cancelled)
    } else {
        Ok(())
    }
}
