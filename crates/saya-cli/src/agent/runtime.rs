use super::extraction_trace::trace_extraction;
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
    let inputs = prepare_turn(runtime, &overrides, can_prompt)
        .await
        .map_err(with_next_step)?;
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
        runtime.resolved.memory.mode,
        super::system_prompt::memory_reachable(state_db.is_some(), allow_query_data),
    );
    let profile_names: Vec<String> = registry.names().into_iter().map(str::to_string).collect();
    let memory = &runtime.resolved.memory;
    let recall_mode = super::learning::recall_mode_for(memory.mode);
    let (mut context_blocks, receipt) = match recall_mode {
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
                runtime.resolved.ai.context_byte_budget,
            )
            .await
        }
    };
    // The last-SQL hint rides the user turn beside the recall context block —
    // never the system prompt, where it would change on every follow-up that
    // ran SQL and forfeit the provider's prefix cache. Placed after the recall
    // block so it sits adjacent to the user's question.
    if let Some(hint) = last_sql
        .as_deref()
        .and_then(super::system_prompt::last_sql_hint_block)
    {
        context_blocks.push(hint);
    }
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
    // Turn and tool-call ceilings come from the environment and are unbounded
    // when unset: SAYA_AGENT_MAX_TURNS / SAYA_AGENT_MAX_TOOL_CALLS, with no
    // upper limit on a set value.
    let env_budgets = saya_agent::budgets_from_env(|name| std::env::var(name).ok());
    let limits = AgentLimits {
        max_turns: env_budgets.0,
        max_tool_calls: env_budgets.1,
        permit_candidate_writes: learning.permit_candidate_writes,
        context_byte_budget: runtime.resolved.ai.context_byte_budget,
    };
    let mut output = run_agent_with_sink(
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
    // The usage the extraction call reported, if any. Stays `None` when
    // learning is disabled, the gate declined, or the call produced no
    // response (provider error or timeout) — absent is not zero.
    let mut learning_usage: Option<saya_agent::TokenUsage> = None;
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
        let gate = super::learning::ProposalGating::evaluate(
            &turn_record,
            &drained_obs,
            !overridden.is_empty(),
        );
        if gate.is_run() {
            let object_count = turn_record.object_table.len();
            // The answer is already on screen; this call is what the adapter is
            // still waiting on, so say so before starting it.
            sink.emit(AgentEvent::KnowledgeLearningStarted).await;
            let extraction_started = std::time::Instant::now();
            let extraction_res = tokio::time::timeout(
                super::learning::EXTRACTION_TIMEOUT,
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
            let extraction_elapsed = Some(extraction_started.elapsed());

            match extraction_res {
                // Extraction completed: the outcome carries the proposals and
                // the usage the provider reported, even when parsing or
                // ingestion then failed (tokens may have been billed first).
                Ok(outcome) => {
                    learning_usage = outcome.usage;
                    match outcome.dtos {
                        Ok(dtos) => {
                            trace_extraction(
                                "ok",
                                object_count,
                                Some(dtos.len()),
                                None,
                                extraction_elapsed,
                            );
                            for dto in dtos {
                                sink.emit(AgentEvent::knowledge_proposed(dto)).await;
                            }
                        }
                        Err(error) => {
                            trace_extraction(
                                "failed",
                                object_count,
                                Some(0),
                                Some(&error.to_string()),
                                extraction_elapsed,
                            );
                            sink.emit(AgentEvent::knowledge_learning_skipped(
                                saya_agent::LearningSkipReason::Failed,
                            ))
                            .await;
                        }
                    }
                }
                // Timeout fired before extraction returned; same fail-soft rule.
                // No response was produced, so there is no usage to report.
                Err(_) => {
                    trace_extraction("timed_out", object_count, Some(0), None, extraction_elapsed);
                    sink.emit(AgentEvent::knowledge_learning_skipped(
                        saya_agent::LearningSkipReason::TimedOut,
                    ))
                    .await;
                }
            }
        } else {
            // Gate decline stays silent on screen (decision 2); trace it for
            // observability when debugging the boundary.
            trace_extraction(
                "gate_declined",
                turn_record.object_table.len(),
                None,
                None,
                None,
            );
        }
    }

    // Attach the extraction usage to the output so both the TUI and headless
    // recorders can fold it into the learning total. A failed answering call
    // leaves the output as `Err`, so the learning usage is simply dropped.
    if let Ok(out) = output.as_mut() {
        out.learning_usage = learning_usage;
    }

    output.map_err(|error| match error {
        AgentError::Provider(error) => AgentRuntimeError::Provider(provider_message(&error)),
        AgentError::Limit(error) => {
            AgentRuntimeError::Agent(format!("agent limit reached: {error}"))
        }
        AgentError::InvalidToolCall => {
            AgentRuntimeError::Agent("provider returned an unsupported tool call".into())
        }
        AgentError::InvalidHistory => {
            AgentRuntimeError::Agent("conversation history is invalid".into())
        }
        AgentError::Cancelled => AgentRuntimeError::Agent("request cancelled".into()),
    })
}

/// the three first-run failures used to name no next step. Guidance is
/// added at the saya-cli boundary (here), not in the provider-neutral agent
/// crate. A provider that is configured but unreachable is "what is configured
/// did not work", not "nothing is configured" — so the next step is to start
/// the provider or check the endpoint, never to re-run `config init` (telling a
/// user whose gateway is momentarily down to re-init would be worse than
/// saying nothing).
fn provider_message(error: &saya_agent::ProviderError) -> String {
    let message = error.to_string();
    if message.contains("could not reach the provider") {
        format!(
            "{message}\nnext: start your AI provider, or run `saya config doctor` to check \
             the endpoint and base_url."
        )
    } else {
        message
    }
}

/// an unresolvable secret reference is "what is configured did not
/// work" — the profile is there but its password is not. `init` cannot supply
/// a secret, so the next step is setting the env var (doctor lists the
/// unresolved references). Only the secret case is annotated; a genuine
/// connection failure is left untouched rather than given advice that might
/// not fit. Applied at the `prepare_turn` seam so both the `ask` and the
/// interactive paths surface it.
fn with_next_step(error: AgentRuntimeError) -> AgentRuntimeError {
    match error {
        AgentRuntimeError::Database(message)
            if message.contains("secret reference")
                && message.contains("could not be resolved") =>
        {
            AgentRuntimeError::Database(format!(
                "{message}\nnext: set the referenced environment variable (or add it to a \
                 .env.saya file passed with --env-file), then re-run; `saya config doctor` \
                 lists unresolved secrets."
            ))
        }
        other => other,
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
