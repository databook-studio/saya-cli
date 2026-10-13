use super::super::{
    admission::coding, check_cancelled, emit, failed_statements, failure_key, output, tools,
};
use super::state::{TurnOutputs, TurnState};
use crate::{AgentError, AgentEvent, ChatMessage, EffectOutcome, LocalStateEffect, ToolDefinition};

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
        failed,
        last_successful_sql,
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
        coding::admit_tool_batch(Some(owner), &effects, cancellation).await?
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
                let Some(definition) = definitions.iter().find(|d| d.name == call.name) else {
                    return EffectOutcome::Unknown;
                };
                if !completed.executed {
                    EffectOutcome::NotStarted
                } else if completed.succeeded || effect_free(definition) {
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
    for (call, completed) in assistant.tool_calls.iter().zip(results) {
        failed_statements::record_outcome(
            failed,
            last_successful_sql,
            failure_key::key_of(call),
            completed.executed,
            completed.succeeded,
            completed.failure_reason.as_deref(),
        );
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
fn effect_free(definition: &ToolDefinition) -> bool {
    definition.read_only
        && !definition.effect.external_side_effect
        && matches!(
            definition.effect.local_state,
            LocalStateEffect::None | LocalStateEffect::Read
        )
}
