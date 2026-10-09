use super::super::{
    check_cancelled, emit, failed_statements, failure_key, output, tool_policy, tools,
};
use super::state::{TurnOutputs, TurnState};
use crate::{AgentError, AgentEvent, AgentMetric, AgentMetricUseful, ToolCall};

pub(super) async fn run_serial_call(
    state: &TurnState<'_>,
    outputs: &mut TurnOutputs<'_>,
    call: ToolCall,
    cap: usize,
) -> Result<(), AgentError> {
    let TurnState {
        tools,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        ..
    } = *state;
    let TurnOutputs {
        events,
        messages,
        used_bounded_sql_query,
        tool_metadata,
        failed,
        last_successful_sql,
    } = outputs;
    let events = &mut **events;
    let messages = &mut **messages;
    let used_bounded_sql_query = &mut **used_bounded_sql_query;
    let tool_metadata = &mut **tool_metadata;
    let failed = &mut **failed;
    let last_successful_sql = &mut **last_successful_sql;
    if let Some(reason) = tools::invalid_reason(&call, definitions) {
        if call.id.trim().is_empty() {
            return Err(AgentError::InvalidToolCall);
        }
        check_cancelled(cancellation)?;
        // A call to an unknown tool has no declared effect to carry —
        // `None`, never a fabricated one; the renderer claims nothing it
        // cannot prove for a tool that does not exist.
        let effect = definitions
            .iter()
            .find(|definition| definition.name == call.name)
            .map(|definition| definition.effect);
        emit(
            events,
            sink,
            AgentEvent::ToolRequested {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                effect,
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
        let completed = tools::complete_tool(
            call.id,
            serde_json::json!({"error": reason}),
            "tool call failed validation".into(),
            false,
            false,
            cap,
        );
        messages.push(completed.message);
        emit(
            events,
            sink,
            AgentEvent::ToolCompleted {
                name: call.name,
                summary: output::completion_summary(&completed.summary, completed.truncated),
            },
        )
        .await;
        return Ok(());
    }
    // A byte-identical repeat of a call that already failed in this run is
    // refused rather than re-executed: the failure is deterministic at
    // parse/safety time, so re-running it wastes the turn (the benchmark
    // saw one statement re-sent 384 times). Feed the prior error back as
    // the tool result so the model has the information it needs to change
    // approach. This does not fail the turn — the point is to return
    // signal cheaply, not to abort. The key is computed here, before
    // `execute` below moves `call.arguments`, and reused for the outcome
    // recording.
    let key = failure_key::key_of(&call);
    if let Some(prior) = failed.prior_error(&key) {
        check_cancelled(cancellation)?;
        let (result, summary) = failed_statements::refuse_repeat(prior);
        tool_metadata.push(crate::ToolMetadata {
            name: call.name.clone(),
            status: "failed".into(),
            arguments: serde_json::to_string(&call.arguments).unwrap_or_default(),
            result_shape: None,
        });
        let completed = tools::complete_tool(call.id, result, summary.into(), false, false, cap);
        messages.push(completed.message);
        emit(
            events,
            sink,
            AgentEvent::ToolCompleted {
                name: call.name,
                summary: output::completion_summary(&completed.summary, completed.truncated),
            },
        )
        .await;
        return Ok(());
    }
    check_cancelled(cancellation)?;
    // The call was validated above, so the definition — and with it the
    // declared effect the event carries — is known before anything runs.
    let definition = definitions
        .iter()
        .find(|tool| tool.name == call.name)
        .expect("validated");
    emit(
        events,
        sink,
        AgentEvent::ToolRequested {
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            effect: Some(definition.effect),
        },
    )
    .await;
    let (approval_state, refusal_detail) =
        tool_policy::resolve_approval(definition, &call.arguments, approval).await;
    let decision = tool_policy::execution_decision(definition, limits, approval_state);
    let executed = matches!(decision, tool_policy::ExecutionDecision::Allow);
    // Capture the serialized arguments before `execute` moves
    // `call.arguments` — the persisted record carries what the model sent
    // (the statement for a SQL tool), and the value-free result shape is
    // read from `result` after the call resolves. Cell values never reach
    // either; `result_shape_of` reads only `row_count` and `columns`.
    let arguments_json = serde_json::to_string(&call.arguments).unwrap_or_default();
    let (result, summary, succeeded) = if executed {
        check_cancelled(cancellation)?;
        // Indicates a database-row-producing query tool ran.
        if definition.effect.database_data {
            *used_bounded_sql_query = true;
        }
        tools::execute_with_outcome(
            tools,
            &call.name,
            call.arguments,
            Some(definition),
            crate::ToolExecutionContext {
                result_cap: cap,
                cancellation: cancellation.clone(),
            },
        )
        .await
    } else {
        emit(
            events,
            sink,
            AgentEvent::ToolDenied {
                name: call.name.clone(),
                reason: refusal_detail.clone().unwrap_or_else(|| match decision {
                    tool_policy::ExecutionDecision::Deny(reason) => reason.reason().into(),
                    tool_policy::ExecutionDecision::Allow
                    | tool_policy::ExecutionDecision::RequireApproval => {
                        unreachable!("a non-executed call has a denial decision")
                    }
                }),
            },
        )
        .await;
        (
            match refusal_detail {
                Some(detail) => {
                    serde_json::json!({"error":format!("tool call denied by approval policy: {detail}")})
                }
                None => serde_json::json!({"error":"tool call denied by approval policy"}),
            },
            "read-only database tool denied".to_owned(),
            false,
        )
    };
    if executed && succeeded {
        sink.metric(AgentMetric::Useful(AgentMetricUseful::SuccessfulTool))
            .await;
    }
    failed_statements::record_outcome(
        failed,
        last_successful_sql,
        key,
        executed,
        succeeded,
        (!succeeded)
            .then(|| result.get("error").and_then(serde_json::Value::as_str))
            .flatten(),
    );
    let completed = tools::complete_tool(call.id, result, summary, succeeded, executed, cap);
    tool_metadata.push(crate::ToolMetadata {
        name: call.name.clone(),
        status: if executed {
            if !completed.succeeded {
                "failed"
            } else {
                "completed"
            }
        } else {
            "denied"
        }
        .into(),
        arguments: arguments_json,
        result_shape: completed.result_shape,
    });
    messages.push(completed.message);
    if executed {
        check_cancelled(cancellation)?;
        emit(
            events,
            sink,
            AgentEvent::ToolCompleted {
                name: call.name,
                summary: output::completion_summary(&completed.summary, completed.truncated),
            },
        )
        .await;
    }
    Ok(())
}
