use super::knowledge_event::knowledge_supplied_event;
use super::tools;
pub(crate) use super::turn_config::{
    AgentRuntimeError, PromptOverrides, effective_ai, query_data_allowed,
};
use super::turn_inputs::{TurnInputs, prepare_turn};
use crate::{config::runtime::RuntimeConfig, prompt_approval::TerminalApproval};
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentOutput, AgentRequest,
    ApprovalDecider, ApprovalPolicy, CancellationToken, ChatMessage, run_agent_with_sink,
};
use saya_store::SqliteStateStore;
use std::sync::Arc;

/// Production entry point: builds provider + registry from config and executes the turn.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_prompt_with_sink(
    runtime: &RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    overrides: PromptOverrides,
    history: Vec<ChatMessage>,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
    state_db: Option<SqliteStateStore>,
    decider: Option<Arc<dyn ApprovalDecider>>,
    last_sql: Option<String>,
) -> Result<AgentOutput, AgentRuntimeError> {
    let inputs = prepare_turn(runtime, &overrides, can_prompt).await?;
    run_prompt_with_inputs(
        runtime,
        inputs,
        prompt,
        approval,
        can_prompt,
        history,
        sink,
        cancellation,
        state_db,
        decider,
        last_sql,
    )
    .await
}

/// The turn body, injectable for tests via [`TurnInputs`].
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_prompt_with_inputs(
    runtime: &RuntimeConfig,
    inputs: TurnInputs,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    history: Vec<ChatMessage>,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
    state_db: Option<SqliteStateStore>,
    decider: Option<Arc<dyn ApprovalDecider>>,
    last_sql: Option<String>,
) -> Result<AgentOutput, AgentRuntimeError> {
    let ai = inputs.ai;
    let provider = inputs.provider;
    let registry = inputs.registry;
    let allow_query_data = query_data_allowed(ai.provider, ai.allow_data_sharing);

    for (name, reason) in inputs.failures {
        sink.emit(AgentEvent::assistant_text(format!(
            "skipped database '{name}': {reason}\n"
        )))
        .await;
    }

    let system_prompt = super::system_prompt::assemble_system_prompt(
        &registry,
        last_sql.as_deref(),
        runtime.resolved.memory.mode,
        super::system_prompt::memory_reachable(state_db.is_some(), allow_query_data),
    );
    let profile_names: Vec<String> = registry.names().into_iter().map(str::to_string).collect();
    let memory = &runtime.resolved.memory;
    let recall_mode = super::learning::recall_mode_for(memory.mode);
    let (context_blocks, receipt) = match recall_mode {
        None => (
            Vec::new(),
            crate::contracts::RecallReceipt::configured_off(),
        ),
        Some(_) if !allow_query_data => (
            Vec::new(),
            crate::contracts::RecallReceipt::privacy_gate_closed(),
        ),
        Some(mode) => {
            super::recall_context::recall_context_blocks(
                prompt,
                system_prompt.as_deref(),
                allow_query_data,
                mode,
                super::learning::bounds_from(memory),
                &registry,
                state_db.as_ref(),
            )
            .await
        }
    };
    sink.emit(knowledge_supplied_event(&receipt)).await;
    let receipt = Arc::new(receipt);
    let learning = super::learning::LearningSetup::from(memory.mode);
    let observations_log = learning.observations.clone();
    let has_state_store = state_db.is_some();
    let override_log = Arc::new(tools::OverrideLog::new());
    let tools = tools::DatabaseTools::with_learning(
        registry,
        runtime.resolved.max_rows,
        allow_query_data,
        state_db,
        learning.observations,
    )
    .with_supplied_objects(receipt.supplied.iter().map(|c| c.object.clone()).collect())
    .with_recall_receipt(Some(receipt.clone()), Some(override_log.clone()));
    let request = AgentRequest {
        prompt: prompt.into(),
        profile_names,
        model: ai.model.clone(),
        system_prompt,
        history,
        context_blocks,
    };
    let fallback_approval = TerminalApproval::new(approval, can_prompt);
    let approver: &dyn ApprovalDecider = match decider.as_deref() {
        Some(decider) => decider,
        None => &fallback_approval,
    };
    let limits = AgentLimits {
        max_turns: runtime.resolved.max_iterations,
        max_tool_calls: runtime.resolved.max_iterations.saturating_mul(2),
        permit_candidate_writes: learning.permit_candidate_writes,
    };
    let output = run_agent_with_sink(
        &*provider,
        &tools,
        request,
        tools::DatabaseTools::definitions(
            allow_query_data,
            has_state_store,
            limits.permit_candidate_writes,
        ),
        limits,
        approver,
        sink,
        cancellation,
    )
    .await;

    let overridden = override_log.drain();
    if !overridden.is_empty() {
        sink.emit(AgentEvent::knowledge_overridden(overridden.clone()))
            .await;
    }

    // Post-turn structured extraction (Safety Property 1: fail-soft isolation).
    if learning.permit_candidate_writes
        && let Some(store) = tools.state_db()
        && let Ok(out) = output.as_ref()
    {
        let drained_obs = observations_log
            .as_ref()
            .map(|l| l.drain())
            .unwrap_or_default();
        let turn_record = super::learning::TurnRecord::assemble(
            prompt,
            &out.answer,
            tools.registry(),
            &drained_obs,
            Some(&receipt),
            &overridden,
        );

        if super::learning::ProposalGating::evaluate(
            &turn_record,
            &drained_obs,
            !overridden.is_empty(),
        )
        .is_run()
        {
            let extraction_res = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                super::learning::run_extraction(
                    &*provider,
                    &ai.model,
                    &turn_record,
                    tools.registry(),
                    store,
                    &receipt,
                ),
            )
            .await;

            if let Ok(Ok(dtos)) = extraction_res {
                for dto in dtos {
                    sink.emit(AgentEvent::knowledge_proposed(dto)).await;
                }
            }
        }
    }

    output.map_err(|error| match error {
        AgentError::Provider(error) => AgentRuntimeError::Provider(error.to_string()),
        AgentError::Limit(error) => {
            AgentRuntimeError::Agent(format!("agent limit reached: {error}"))
        }
        AgentError::InvalidToolCall => {
            AgentRuntimeError::Agent("provider returned an unsupported tool call".into())
        }
        AgentError::InvalidHistory => {
            AgentRuntimeError::Agent("conversation history is invalid".into())
        }
        AgentError::ContextLimit => {
            AgentRuntimeError::Agent("conversation context exceeds the safe limit".into())
        }
        AgentError::Cancelled => AgentRuntimeError::Agent("request cancelled".into()),
    })
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
