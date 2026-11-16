use super::super::{
    admission::coding, check_cancelled, emit, failed_statements, failure_key, output,
    tool_diagnostics, tools,
};
use super::state::{TurnOutputs, TurnState};
use crate::{
    AgentError, AgentEvent, ChatMessage, EffectOutcome, LocalStateEffect, ToolDefinition,
    ToolDiagnostic, ToolDiagnosticStatus,
};

pub(super) async fn run_batch(
    state: &TurnState<'_>,
    outputs: &mut TurnOutputs<'_>,
    assistant: &ChatMessage,
) -> Result<(), AgentError> {
    let TurnState {
        tools,
        definitions,
        result_caps,
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
    let permits = if let Some(owner) = effect_admission {
        let effects = assistant
            .tool_calls
            .iter()
            .map(|call| {
                definitions
                    .iter()
                    .find(|definition| definition.name == call.name)
                    .expect("the batch precondition validated the call")
                    .effect
            })
            .collect::<Vec<_>>();
        match coding::admit_tool_batch(Some(owner), &effects, cancellation).await {
            Ok(permits) => permits,
            Err(error) => {
                tool_diagnostics::refused(&assistant.tool_calls, definitions, sink).await;
                return Err(error);
            }
        }
    } else {
        None
    };
    for call in &assistant.tool_calls {
        // The batch precondition validated every call against the
        // definitions, so the declared effect is at hand for the event —
        // the renderer derives its label from this declaration, the same
        // one the gates below consult.
        let definition = definitions
            .iter()
            .find(|definition| definition.name == call.name)
            .expect("the batch precondition validated the call");
        sink.observe_tool(ToolDiagnostic::new(
            Some(definition),
            ToolDiagnosticStatus::Requested,
        ))
        .await;
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
        if definition.effect.database_data {
            *used_bounded_sql_query = true;
        }
    }
    let results = tools::execute_batch(
        tools,
        &assistant.tool_calls,
        definitions,
        result_caps,
        cancellation,
        sink,
    )
    .await;
    if effect_admission.is_some() {
        let outcomes = results
            .iter()
            .zip(assistant.tool_calls.iter())
            .map(|(completed, call)| {
                if !completed.executed {
                    EffectOutcome::NotStarted
                } else if completed.succeeded
                    || completed.effect_free
                    || definitions
                        .iter()
                        .find(|definition| definition.name == call.name)
                        .is_some_and(effect_free)
                {
                    EffectOutcome::Succeeded
                } else {
                    EffectOutcome::Unknown
                }
            })
            .collect::<Vec<_>>();
        coding::settle_tool_batch(
            effect_admission,
            permits.as_deref(),
            &outcomes,
            cancellation,
        )
        .await?;
    }
    let observations = results
        .iter()
        .filter(|completed| completed.executed)
        .map(|completed| &completed.observation)
        .collect::<Vec<_>>();
    checkpoint.observe_batch(&observations);
    for ((call, mut completed), &cap) in assistant
        .tool_calls
        .iter()
        .zip(results)
        .zip(result_caps.iter())
    {
        let repeat_note = failed_statements::record_outcome(
            failed,
            last_successful_sql,
            failure_key::key_of(call),
            completed.executed,
            completed.succeeded,
        );
        if let Some(note) = repeat_note {
            completed.append_note(note, cap);
        }
        if completed.executed
            && let Some(classifier) = progress.as_mut()
        {
            let observed = classifier.observe(super::super::progress::ToolObservation {
                name: &call.name,
                arguments: &call.arguments,
                executed: true,
                succeeded: completed.succeeded,
                result: &completed.result,
            });
            if let Some(note) = observed.note {
                completed.append_note(&note, cap);
            }
        }
        tool_metadata.push(crate::ToolMetadata {
            name: call.name.clone(),
            status: if completed.succeeded {
                "completed"
            } else {
                "failed"
            }
            .into(),
            arguments: serde_json::to_string(&call.arguments).unwrap_or_default(),
            result_shape: completed.result_shape,
        });
        messages.push(completed.message);
        if completed.executed {
            emit(
                events,
                sink,
                AgentEvent::ToolCompleted {
                    name: call.name.clone(),
                    summary: output::completion_summary(&completed.summary, completed.truncated),
                },
            )
            .await;
        }
    }
    Ok(())
}

/// A failed effect-free call could not have changed anything, so the error the
/// model received is the whole outcome — the settlement stays known, not uncertain.
pub(super) fn effect_free(definition: &ToolDefinition) -> bool {
    definition.read_only
        && !definition.effect.external_side_effect
        && matches!(
            definition.effect.local_state,
            LocalStateEffect::None | LocalStateEffect::Read
        )
}
