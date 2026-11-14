use super::super::{failed_statements, tool_diagnostics, tool_policy, tools};
use super::state::{TurnOutputs, TurnState};
use super::{batch, serial};
use crate::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, ApprovalDecider, CancellationToken,
    ChatMessage, EffectAdmission, ToolConcurrency, ToolDefinition, ToolExecutor,
};

#[allow(clippy::too_many_arguments)]
pub(in crate::loop_runner) async fn run_turn_tools(
    tools: &dyn ToolExecutor,
    assistant: ChatMessage,
    definitions: &[ToolDefinition],
    result_caps: &[usize],
    limits: &AgentLimits,
    approval: &dyn ApprovalDecider,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    effect_admission: Option<&dyn EffectAdmission>,
    events: &mut Vec<AgentEvent>,
    messages: &mut Vec<ChatMessage>,
    used_bounded_sql_query: &mut bool,
    tool_metadata: &mut Vec<crate::ToolMetadata>,
    checkpoint: &mut super::super::coding_checkpoint::CodingCheckpoint,
    failed: &mut failed_statements::FailedStatements,
    last_successful_sql: &mut Option<String>,
    progress: &mut Option<super::super::progress::ProgressClassifier>,
) -> Result<(), AgentError> {
    let state = TurnState {
        tools,
        definitions,
        result_caps,
        limits,
        approval,
        sink,
        cancellation,
        effect_admission,
    };
    let mut outputs = TurnOutputs {
        events,
        messages,
        used_bounded_sql_query,
        tool_metadata,
        checkpoint,
        failed,
        last_successful_sql,
        progress,
    };
    // A batch overlaps only when every member opted in and is allowed without
    // a per-call approval. Prompted calls stay serial, preserving approval
    // authority and prompt order.
    let batch_parallel = assistant.tool_calls.len() > 1
        && assistant.tool_calls.iter().all(|call| {
            tools::invalid_reason(call, state.definitions).is_none()
                && state
                    .definitions
                    .iter()
                    .find(|definition| definition.name == call.name)
                    .is_some_and(|definition| {
                        definition.concurrency == ToolConcurrency::Concurrent
                            && matches!(
                                tool_policy::execution_decision(
                                    definition,
                                    state.limits,
                                    tool_policy::ApprovalState::NotRequired,
                                ),
                                tool_policy::ExecutionDecision::Allow
                            )
                    })
        });
    if batch_parallel {
        return batch::run_batch(&state, &mut outputs, &assistant).await;
    }
    let mut pending = assistant.tool_calls.into_iter();
    for cap in state.result_caps.iter().copied() {
        let Some(call) = pending.next() else { break };
        if let Err(error) = serial::run_serial_call(&state, &mut outputs, call, cap).await {
            tool_diagnostics::refused(pending.as_slice(), definitions, sink).await;
            return Err(error);
        }
    }
    Ok(())
}
