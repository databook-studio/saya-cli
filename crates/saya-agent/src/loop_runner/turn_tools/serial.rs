use super::super::tool_outcome::ToolOutcome;
use super::super::{
    admission::coding, check_cancelled, emit, failed_statements, failure_key, output, tool_policy,
    tools,
};
use super::state::{TurnOutputs, TurnState};
use crate::{
    AgentError, AgentEvent, AgentMetric, AgentMetricUseful, ToolCall, ToolDiagnostic,
    ToolDiagnosticStatus,
};

pub(super) async fn run_serial_call(
    state: &TurnState<'_>,
    outputs: &mut TurnOutputs<'_>,
    call: ToolCall,
    cap: usize,
) -> Result<(), AgentError> {
    if super::refusals::handle(state, outputs, &call, cap).await? {
        return Ok(());
    }
    let TurnState {
        tools,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        effect_admission,
        ..
    } = *state;
    let TurnOutputs {
        events,
        messages,
        used_bounded_sql_query,
        tool_metadata,
        checkpoint,
        failed,
        last_successful_sql,
        progress,
    } = outputs;
    let events = &mut **events;
    let messages = &mut **messages;
    let used_bounded_sql_query = &mut **used_bounded_sql_query;
    let tool_metadata = &mut **tool_metadata;
    let failed = &mut **failed;
    let last_successful_sql = &mut **last_successful_sql;
    check_cancelled(cancellation)?;
    let key = failure_key::key_of(&call);
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
    let (approval_state, refusal_detail, immutable_authority_refusal) =
        tool_policy::resolve_approval(definition, &call.arguments, approval).await;
    let decision = tool_policy::execution_decision(definition, limits, approval_state);
    let executed = matches!(decision, tool_policy::ExecutionDecision::Allow);
    let permit = if executed {
        match coding::admit_tool(effect_admission, definition.effect, cancellation).await {
            Ok(permit) => permit,
            Err(error) => {
                let arguments_json = call.arguments.to_string();
                sink.observe_tool(
                    ToolDiagnostic::new(Some(definition), ToolDiagnosticStatus::Refused)
                        .with_failure_detail(&arguments_json, || error.to_string())
                        .with_transcript(crate::ToolCallTranscript::new(
                            &call.id,
                            &arguments_json,
                            None,
                            0,
                        )),
                )
                .await;
                return Err(error);
            }
        }
    } else {
        None
    };
    // Capture the serialized arguments before `execute` moves
    // `call.arguments` — the persisted record carries what the model sent
    // (the statement for a SQL tool), and the value-free result shape is
    // read from `result` after the call resolves. Cell values never reach
    // either; `result_shape_of` reads only `row_count` and `columns`.
    let arguments_json = serde_json::to_string(&call.arguments).unwrap_or_default();
    // The classifier reads the canonical argument value, so keep it across
    // the move into the executor.
    let observation_arguments = call.arguments.clone();
    // How long this call took, from the loop's own view: the policy decision,
    // the admission gates and the execution it decided.
    let started = std::time::Instant::now();
    let outcome = if executed {
        if let Err(error) =
            coding::check_tool_dispatch(effect_admission, permit, cancellation).await
        {
            let arguments_json = call.arguments.to_string();
            sink.observe_tool(
                ToolDiagnostic::new(Some(definition), ToolDiagnosticStatus::Refused)
                    .with_failure_detail(&arguments_json, || error.to_string())
                    .with_transcript(crate::ToolCallTranscript::new(
                        &call.id,
                        &arguments_json,
                        None,
                        0,
                    )),
            )
            .await;
            return Err(error);
        }
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
        if let tool_policy::ExecutionDecision::Deny(reason) = decision
            && let Some(class) = reason.immutable_authority_class()
        {
            checkpoint.observe_authority_denial(&call.name, class);
        }
        if let Some(refusal) = immutable_authority_refusal {
            checkpoint.observe_authority_denial(&call.name, refusal.class());
        }
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
        ToolOutcome::refused(
            match refusal_detail {
                Some(detail) => {
                    serde_json::json!({"error":format!("tool call denied by approval policy: {detail}")})
                }
                None => serde_json::json!({"error":"tool call denied by approval policy"}),
            },
            "read-only database tool denied".to_owned(),
        )
    };
    let status = if executed {
        ToolDiagnostic::result_status(&outcome.result, outcome.succeeded)
    } else {
        ToolDiagnosticStatus::Denied
    };
    // The reason is the exact text this call's tool result carries — the rule
    // that denied it, or the executor's own error — and it is read lazily so a
    // succeeding result is never serialized just to be discarded.
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let result_json = serde_json::to_string(&outcome.result).unwrap_or_default();
    sink.observe_tool(
        ToolDiagnostic::new(Some(definition), status)
            .with_failure_detail(&arguments_json, || {
                ToolDiagnostic::result_error_text(&outcome.result)
            })
            .with_transcript(crate::ToolCallTranscript::new(
                &call.id,
                &arguments_json,
                Some(&result_json),
                duration_ms,
            )),
    )
    .await;
    if executed {
        // Same rule as the concurrent batch: a failed call is a known one when the executor
        // proved it changed nothing or the tool is effect-free.
        let known =
            outcome.succeeded || outcome.effect_free || super::batch::effect_free(definition);
        coding::settle_tool_result(effect_admission, permit, known, cancellation).await?;
    }
    if executed && outcome.succeeded {
        sink.metric(AgentMetric::Useful(AgentMetricUseful::SuccessfulTool))
            .await;
    }
    let repeat_note = failed_statements::record_outcome(
        failed,
        last_successful_sql,
        key,
        executed,
        outcome.succeeded,
    );
    if executed && !cancellation.is_cancelled() {
        checkpoint.observe_serial(&outcome.observation);
    }
    let succeeded = outcome.succeeded;
    let mut completed = tools::complete_tool(call.id, outcome, executed, cap);
    if let Some(note) = repeat_note {
        completed.append_note(note, cap);
    }
    if executed && let Some(classifier) = progress.as_mut() {
        let observed = classifier.observe(super::super::progress::ToolObservation {
            name: &call.name,
            arguments: &observation_arguments,
            executed,
            succeeded,
            result: &completed.result,
        });
        if let Some(note) = observed.note {
            completed.append_note(&note, cap);
        }
    }
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
