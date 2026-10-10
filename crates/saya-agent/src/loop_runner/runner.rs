use crate::{
    AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken, ChatMessage, ChatProvider,
    EffectAdmission, TokenUsage, ToolDefinition, ToolExecutor,
};

use super::{
    AgentError, AgentLimits, AgentOutput, PartialOutput, admission, budget_output, control,
    salvage, turn,
};

pub(super) async fn run_agent_inner(
    invocation: control::AgentRunInvocation<'_>,
) -> Result<AgentOutput, AgentError> {
    if let Some(scope) = invocation.answering.clone() {
        let mut admission = scope.lock().await?;
        admission.begin_invocation();
        run_agent_loop(invocation, &mut admission).await
    } else {
        let mut admission = admission::Admission::new(super::AnsweringLimits::default(), false);
        run_agent_loop(invocation, &mut admission).await
    }
}

async fn run_agent_loop(
    invocation: control::AgentRunInvocation<'_>,
    admission: &mut admission::Admission,
) -> Result<AgentOutput, AgentError> {
    let control::AgentRunInvocation {
        provider,
        tools,
        request,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        answering,
        request_preflight,
        effect_admission,
    } = invocation;
    let controlled = answering.is_some();
    admission.initialize(sink).await;
    if controlled {
        admission.check(&cancellation)?;
    }
    let (messages, ordinary_history_pairs, protected_prefix_len) =
        crate::history::build_messages_with_history_count(
            request.system_prompt.as_deref(),
            &request.context_blocks,
            &request.prompt,
            &request.history,
            limits.context_byte_budget,
        )?;
    let mut state = RunState {
        provider,
        tools,
        request,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        request_preflight,
        effect_admission,
        messages,
        ordinary_history_pairs,
        protected_prefix_len,
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        continuation_count: 0,
        pending_continuation: None,
        // Calls that failed during this run — SQL statements keyed on the
        // statement exactly as submitted, other tools on (tool, arguments) — so a
        // byte-identical re-submission is refused rather than re-executed (loop
        // invariant for the "do not repeat a failed call" advice the model does
        // not always obey).
        failed: super::failed_statements::FailedStatements::new(),
        // The last statement that completed successfully, so a run that exhausts its
        // budget without nominating can still surface its best available answer.
        last_successful_sql: None,
        designation: super::designation::State::new(),
    };
    loop {
        if let Err(error) = admission.check(&state.cancellation) {
            if let Some(dimension) = admission.stopped_dimension(&error, &state.cancellation) {
                return Ok(budget_output(
                    dimension,
                    &mut state.events,
                    state.sink,
                    PartialOutput {
                        usage: state.usage,
                        used_bounded_sql_query: state.used_bounded_sql_query,
                        tool_metadata: state.tool_metadata,
                        answer_sql: state.designation.sql.clone().or(state.last_successful_sql),
                        answer: String::new(),
                    },
                )
                .await);
            }
            return Err(error);
        }
        if admission.legacy.turn_limit(state.limits.max_turns) {
            return salvage::salvage(
                state.provider,
                &state.request.model,
                &mut state.messages,
                &mut state.ordinary_history_pairs,
                &mut state.protected_prefix_len,
                &[],
                state.limits.context_byte_budget,
                state.request_preflight,
                state.sink,
                &state.cancellation,
                &mut state.events,
                &mut state.usage,
                admission,
                state.used_bounded_sql_query,
                state.tool_metadata,
                state.designation.sql.clone().or(state.last_successful_sql),
            )
            .await;
        }
        if let Some(output) = turn::run(&mut state, admission).await? {
            return Ok(output);
        }
    }
}

pub(super) struct RunState<'a> {
    pub(super) provider: &'a dyn ChatProvider,
    pub(super) tools: &'a dyn ToolExecutor,
    pub(super) request: AgentRequest,
    pub(super) definitions: Vec<ToolDefinition>,
    pub(super) limits: AgentLimits,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
    pub(super) request_preflight: Option<super::RequestPreflightConfig>,
    pub(super) effect_admission: Option<&'a dyn EffectAdmission>,
    pub(super) messages: Vec<ChatMessage>,
    pub(super) ordinary_history_pairs: usize,
    pub(super) protected_prefix_len: usize,
    pub(super) events: Vec<crate::AgentEvent>,
    pub(super) used_bounded_sql_query: bool,
    pub(super) tool_metadata: Vec<crate::ToolMetadata>,
    pub(super) usage: TokenUsage,
    pub(super) continuation_count: u32,
    pub(super) pending_continuation: Option<super::receive::ContinuationProgress>,
    pub(super) failed: super::failed_statements::FailedStatements,
    pub(super) last_successful_sql: Option<String>,
    pub(super) designation: super::designation::State,
}
