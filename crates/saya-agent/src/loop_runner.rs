#[path = "agent_output.rs"]
mod output;
#[path = "loop_receive.rs"]
mod receive;
#[path = "loop_tools.rs"]
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
        for call in assistant.tool_calls {
            tools::check_call(&call, &definitions)?;
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
                },
            )
            .await;
            let definition = definitions
                .iter()
                .find(|tool| tool.name == call.name)
                .expect("validated");
            let approved = !definition.requires_approval || approval.approve(definition).await;
            let (result, summary) = if approved {
                check_cancelled(&cancellation)?;
                if call.name == "bounded_sql_query" {
                    used_bounded_sql_query = true;
                }
                tools::execute(tools, &call.name, call.arguments).await
            } else {
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolDenied {
                        name: call.name.clone(),
                        reason: "approval was not granted".into(),
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
                status: if approved {
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
            if approved {
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
