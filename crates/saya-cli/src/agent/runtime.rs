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

/// The production entry: builds the provider + registry from config (via
/// [`super::turn_inputs::prepare_turn`]), then runs the turn. Callers are
/// unchanged (the build stays inside this future, so ctrl-c still covers the
/// connect).
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

/// The turn body, injectable for tests via [`TurnInputs`]. Emits
/// [`AgentEvent::KnowledgeSupplied`] on `sink` immediately after recall is
/// assembled and **before** `run_agent_with_sink` is called (spec P1b §2) —
/// the moment matters more than the event: by the time the answer exists the
/// claim has already shaped the SQL, so a receipt that arrives then is a
/// changelog, not a control.
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

    let system_prompt = {
        let base = registry.describe_context();
        match last_sql {
            Some(sql) if !sql.trim().is_empty() => {
                let hint = format!(
                    "For context, the most recent SQL you ran was:\n{sql}\n\nIf the user's request \
                     refines, filters, sorts, or drills into that previous result, adapt this query \
                     instead of rediscovering the schema from scratch."
                );
                Some(match base {
                    Some(b) => format!("{b}\n\n{hint}"),
                    None => hint,
                })
            }
            _ => base,
        }
    };
    let profile_names: Vec<String> = registry.names().into_iter().map(str::to_string).collect();
    let memory = &runtime.resolved.memory;
    // Recall mode from `[memory] mode`. `Off` skips recall entirely — no
    // store query, no block (spec E §1). The privacy gate (`allow_query_data`)
    // is independent and still skips recall when sharing is off regardless of
    // `mode` (spec E §4).
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
    // The emit is the point of this slice: one `KnowledgeSupplied` per turn,
    // before any provider request, naming what recall supplied (or why it did
    // not run). Emitting must never fail the turn — `sink.emit` is infallible
    // and the mapping is pure, so the prompt still runs regardless (spec §3).
    sink.emit(knowledge_supplied_event(&receipt)).await;
    // A1: the receipt is shared with the override detector, which matches a
    // statement's references against the confirmed claims recall supplied. An
    // `Arc` so the tools hold one reference for per-statement detection while
    // `with_supplied_objects` reads the supplied object names here.
    let receipt = Arc::new(receipt);
    // Learning mode → write permission + observation-log attachment (spec E).
    // `Off` attaches nothing; `assisted` attaches a log for recording observations.
    let learning = super::learning::LearningSetup::from(memory.mode);
    // Capture before `state_db` moves into the tools; the contract tools are
    // advertised only when a store is present (spec 2b-3a §3).
    let has_state_store = state_db.is_some();
    // P2d: a request-scoped log `contract_propose` records a *persisted*
    // candidate claim into. Attached only when a store is present (a proposal
    // can persist nowhere else); draining an empty log is a no-op. The runtime
    // keeps its own `Arc` handle so it can drain after `tools` consumes its
    // clone — the same shared-handle pattern `observation_log` uses.
    let proposed_claims_log =
        has_state_store.then(|| std::sync::Arc::new(tools::ProposedClaimsLog::new()));
    // A1: a request-scoped log the query tools record override findings into as
    // each statement runs. Always attached — detection is independent of the
    // learning mode (a confirmed claim being contradicted is a fact about the
    // turn whether or not learning collects evidence), so the log is not gated
    // on `learning.observations`. The runtime keeps its own `Arc` handle so it
    // can drain after `tools` consumes its clone — the same shared-handle
    // pattern `proposed_claims_log` uses.
    let override_log = Arc::new(tools::OverrideLog::new());
    let tools = tools::DatabaseTools::with_learning(
        registry,
        runtime.resolved.max_rows,
        allow_query_data,
        state_db,
        learning.observations,
        proposed_claims_log.clone(),
    )
    .with_supplied_objects(receipt.supplied.iter().map(|c| c.object.clone()).collect())
    .with_recall_receipt(Some(receipt.clone()), Some(override_log.clone()));
    let request = AgentRequest {
        prompt: prompt.into(),
        profile_names,
        model: ai.model,
        system_prompt,
        history,
        context_blocks,
    };
    // Use the caller-supplied decider (e.g. the TUI approval modal) when present,
    // otherwise the terminal prompt/policy decider.
    let fallback_approval = TerminalApproval::new(approval, can_prompt);
    let approver: &dyn ApprovalDecider = match decider.as_deref() {
        Some(decider) => decider,
        None => &fallback_approval,
    };
    // `permit_candidate_writes` is true only for `auto-candidate`; `suggest`
    // and `off` leave it false so `contract_propose` is hidden (definitions) and
    // denied (loop guard) in agreement (spec 4b §2). Changing the mode applies
    // on the next turn: this flag is read once per `run_prompt_with_sink` call.
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
    // P2d: drain persisted proposals log and emit KnowledgeProposed per recorded claim.
    if let Some(log) = proposed_claims_log.as_ref() {
        for claim in log.drain() {
            sink.emit(AgentEvent::knowledge_proposed(claim)).await;
        }
    }
    // A1: drain override log and emit KnowledgeOverridden if detector raised findings.
    let overridden = override_log.drain();
    if !overridden.is_empty() {
        sink.emit(AgentEvent::knowledge_overridden(overridden))
            .await;
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
