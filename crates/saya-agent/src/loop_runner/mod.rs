mod output;
mod receive;
mod tools;

use crate::{
    AgentEvent, AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken, ChatProvider,
    ToolDefinition, ToolExecutor,
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
    for _ in 0..limits.max_turns {
        check_cancelled(&cancellation)?;
        let assistant = receive::receive(
            provider,
            &request.model,
            &messages,
            &definitions,
            sink,
            &cancellation,
            &mut events,
        )
        .await?;
        messages.push(assistant.clone());
        if assistant.tool_calls.is_empty() {
            check_cancelled(&cancellation)?;
            emit(&mut events, sink, AgentEvent::Complete).await;
            return Ok(AgentOutput {
                answer: assistant.content,
                events,
                used_bounded_sql_query,
                tool_metadata,
            });
        }
        // When every call in the message is valid and auto-runnable (no
        // approval gate, no external side effect, no denied candidate write),
        // they are independent: run them concurrently instead of paying
        // their latency sequentially.
        let batch_parallel = assistant.tool_calls.len() > 1
            && assistant.tool_calls.iter().all(|call| {
                tools::invalid_reason(call, &definitions).is_none()
                    && definitions.iter().any(|definition| {
                        definition.name == call.name
                            && !definition.effect.requires_approval
                            && !definition.effect.external_side_effect
                            && !(definition.effect.local_state
                                == crate::LocalStateEffect::WriteCandidate
                                && !limits.permit_candidate_writes)
                    })
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
                tool_metadata.push(crate::ToolMetadata {
                    name: call.name.clone(),
                    status: if summary.contains("failed") {
                        "failed"
                    } else {
                        "completed"
                    }
                    .into(),
                });
                messages.push(tools::tool_message(call.id.clone(), result));
                check_cancelled(&cancellation)?;
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolCompleted {
                        name: call.name.clone(),
                        summary: summary.into(),
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
                messages.push(tools::tool_message(
                    call.id,
                    serde_json::json!({"error": reason}),
                ));
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
            // Fail closed: a tool that may write a candidate claim is refused
            // unless the runner was constructed with candidate writes
            // permitted. The default is not permitted, so registering a
            // `WriteCandidate` tool in a later slice cannot silently start
            // writing. This denies rather than executes, so the turn
            // continues and the model sees a `ToolDenied` event with a reason.
            let candidate_denied = definition.effect.local_state
                == crate::LocalStateEffect::WriteCandidate
                && !limits.permit_candidate_writes;
            let executed = approved && !candidate_denied;
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
                        reason: if candidate_denied {
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
            messages.push(tools::tool_message(call.id, result));
            if executed {
                check_cancelled(&cancellation)?;
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolCompleted {
                        name: call.name,
                        summary: summary.into(),
                    },
                )
                .await;
            }
        }
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
