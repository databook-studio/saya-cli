use crate::ChatMessage;

use super::{AgentError, PartialOutput, admission, runner::RunState, salvage, turn_tools};

pub(super) async fn execute(
    state: &mut RunState<'_>,
    admission: &mut admission::Admission,
    assistant: ChatMessage,
    result_caps: Vec<usize>,
    response_limits: super::AgentLimits,
) -> Result<Option<super::AgentOutput>, AgentError> {
    // The tool-call ceiling is a whole-run total checked once per turn,
    // before any of the turn's calls run: if this turn would cross it, the
    // run stops rather than executing a partial batch.
    if admission
        .legacy
        .tool_limit(state.limits.max_tool_calls, assistant.tool_calls.len())
    {
        return Ok(Some(
            salvage::salvage(
                state.provider,
                &state.request.model,
                &mut state.messages,
                &mut state.ordinary_history_pairs,
                &mut state.protected_prefix_len,
                &assistant.tool_calls,
                state.limits.context_byte_budget,
                state.request_preflight,
                state.sink,
                &state.cancellation,
                &mut state.events,
                &mut state.usage,
                admission,
                state.used_bounded_sql_query,
                std::mem::take(&mut state.tool_metadata),
                state
                    .designation
                    .sql
                    .clone()
                    .or(state.last_successful_sql.clone()),
                state.effect_admission,
                state.require_physical_effect_admission,
            )
            .await?,
        ));
    }
    admission.legacy.admit_tools(assistant.tool_calls.len());
    let partial_answer = assistant.content.clone();
    let tool_result = turn_tools::run_turn_tools(
        state.tools,
        assistant,
        &state.definitions,
        &result_caps,
        &response_limits,
        state.approval,
        state.sink,
        &state.cancellation,
        state.effect_admission,
        &mut state.events,
        &mut state.messages,
        &mut state.used_bounded_sql_query,
        &mut state.tool_metadata,
        &mut state.failed,
        &mut state.last_successful_sql,
    )
    .await;
    if let Err(error) = tool_result {
        let error = if matches!(&error, AgentError::Cancelled) {
            admission.check(&state.cancellation).err().unwrap_or(error)
        } else {
            error
        };
        if let Some(dimension) = admission.stopped_dimension(&error, &state.cancellation) {
            return Ok(Some(
                super::budget_output(
                    dimension,
                    &mut state.events,
                    state.sink,
                    PartialOutput {
                        usage: state.usage,
                        used_bounded_sql_query: state.used_bounded_sql_query,
                        tool_metadata: std::mem::take(&mut state.tool_metadata),
                        answer_sql: state
                            .designation
                            .sql
                            .clone()
                            .or(state.last_successful_sql.clone()),
                        answer: partial_answer,
                    },
                )
                .await,
            ));
        }
        return Err(error);
    }
    // Intra-loop context budget: the pre-loop trim bounds history, but
    // assistant turns and tool results accumulate here. Trim every turn —
    // batch (parallel) and sequential alike — so the byte budget binds
    // unconditionally, never just on the turns a single call ran.
    super::output::trim_to_budget(
        &mut state.messages,
        state.limits.context_byte_budget,
        state.protected_prefix_len,
    );
    Ok(None)
}
