use super::knowledge_event::knowledge_supplied_event;
use super::tools;
#[cfg(test)]
pub(crate) use super::turn_config::query_data_allowed;
pub(crate) use super::turn_config::{
    AgentRuntimeError, PromptOverrides, effective_ai, query_data_allowed_for_endpoint,
};
use super::turn_inputs::{TurnInputs, prepare_turn};
use crate::interactive::session_universe::SessionUniverse;
use crate::{
    config::runtime::RuntimeConfig, grant_token::TurnPrimary, prompt_approval::TerminalApproval,
};
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentMode, AgentOutput, AgentRequest,
    ApprovalDecider, ApprovalPolicy, CancellationToken, ChatMessage, LocalStateEffect,
    run_agent_with_sink,
};
use saya_store::SqliteStateStore;
use std::sync::Arc;

/// Production entry point: builds provider + registry from config and executes the turn.
///
/// `can_prompt` means "this surface may read stdin" — it feeds the
/// connector's secret prompt, the turn's fallback [`TerminalApproval`], and
/// nothing else. `can_obtain_approval` means "this surface can obtain a
/// per-call approval at all" — it feeds the advertisement gate
/// ([`SessionUniverse::definitions`] for sessions, the one-shot branch
/// below for `ask`), never the stdin fallback. The line REPL passes its
/// live-terminal fact for both; the TUI passes `false` for the first and
/// `true` for the second (its modal); the one-shot `ask` path passes the
/// same value for both (unchanged behaviour). Keep them split.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_prompt_with_sink(
    runtime: &RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    can_obtain_approval: bool,
    overrides: PromptOverrides,
    history: Vec<ChatMessage>,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
    state_db: Option<SqliteStateStore>,
    decider: Option<Arc<dyn ApprovalDecider>>,
    last_sql: Option<String>,
    // The session's tool universe, composed once per interactive session.
    // `None` (the one-shot `ask` path) leaves `workspace_read` denying with
    // a typed error and the write-shaped tools hidden.
    session: Option<Arc<SessionUniverse>>,
    // The agent's task posture, threaded like `approval`: the session's
    // `/mode` state at the composition root.
    agent_mode: AgentMode,
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
        can_obtain_approval,
        history,
        sink,
        cancellation,
        state_db,
        decider,
        last_sql,
        session,
        agent_mode,
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
    can_obtain_approval: bool,
    history: Vec<ChatMessage>,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
    state_db: Option<SqliteStateStore>,
    decider: Option<Arc<dyn ApprovalDecider>>,
    last_sql: Option<String>,
    // The session's tool universe, composed once per interactive session.
    // `None` (the one-shot `ask` path) leaves `workspace_read` denying with
    // a typed error and the write-shaped tools hidden.
    session: Option<Arc<SessionUniverse>>,
    // The agent's task posture, threaded like `approval`.
    agent_mode: AgentMode,
) -> Result<AgentOutput, AgentRuntimeError> {
    let ai = inputs.ai;
    let provider = inputs.provider;
    let registry = inputs.registry;
    let allow_query_data =
        query_data_allowed_for_endpoint(ai.provider, ai.base_url.as_deref(), ai.allow_data_sharing);

    // The turn's registry is the turn's connection fact: the deciders hold
    // the session universe's primary handle, and this binds the turn's
    // primary into it — before the registry moves into the tools. The
    // fallback decider (the one-shot `ask` path, no decider passed in)
    // owns its own handle, bound the same way, so its SQL suggestions name
    // the turn's real primary.
    let fallback_primary = TurnPrimary::default();
    if let Some(session) = session.as_ref() {
        session.primary.bind(&registry);
    } else {
        fallback_primary.bind(&registry);
    }

    for (name, reason) in inputs.failures {
        sink.emit(AgentEvent::assistant_text(format!(
            "skipped database '{name}': {reason}\n"
        )))
        .await;
    }

    // Session-stable facts: the turn's connection set plus the session's
    // bound root, held unchanged across the turns of one session so the
    // system block keeps one prefix-cache key. The root borrows from the
    // session universe for exactly this call — the facts render now, and no
    // reference escapes into the request.
    let root;
    let facts = match session.as_ref() {
        Some(universe) => {
            root = universe.root().map(std::path::Path::to_path_buf);
            super::session_facts::SessionFacts {
                registry: &registry,
                workspace_root: root.as_deref(),
            }
        }
        None => super::session_facts::SessionFacts {
            registry: &registry,
            workspace_root: None,
        },
    };
    // The plain entry point is the same assembly over an empty session
    // (no workspace root); the mode-aware entry point takes the session
    // facts. Both stay live: the session-aware prompt is what the turn
    // sends, and the empty-session prompt is the no-root shape the pins
    // read. Debug-assert they agree when no root binds — same inputs,
    // same bytes — so the two shapes cannot drift.
    let reachable = super::system_prompt::memory_reachable(state_db.is_some(), allow_query_data);
    let empty_prompt = super::system_prompt::assemble_system_prompt(
        &registry,
        runtime.resolved.memory.mode,
        reachable,
    );
    let system_prompt = super::system_prompt::assemble_system_prompt_for_mode(
        &registry,
        runtime.resolved.memory.mode,
        reachable,
        &facts,
        agent_mode,
    );
    debug_assert!(
        facts.workspace_root.is_some()
            || agent_mode != saya_agent::AgentMode::Build
            || system_prompt == empty_prompt
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
    // The session task list rides the same lane, last: only when non-empty,
    // so an empty list costs zero tokens. The block is the live cell's own
    // rendering (see `session_tasks_render`), quoted as data through the
    // same untrusted lane — never the system prompt, which stays
    // byte-identical whether or not a list exists.
    if let Some(session) = session.as_ref()
        && let Some(block) = super::super::interactive::session_tasks_render::render_tasks_block(
            &session.tasks().current(),
        )
    {
        context_blocks.push(block);
    }
    sink.emit(knowledge_supplied_event(&receipt)).await;
    let receipt = Arc::new(receipt);
    let learning = super::learning::LearningSetup::from(memory.mode);
    let observations_log = learning.observations.clone();
    let has_state_store = state_db.is_some();
    let override_log = Arc::new(tools::OverrideLog::new());
    let database = Arc::new(
        tools::DatabaseTools::with_learning(
            registry,
            runtime.resolved.max_rows,
            allow_query_data,
            state_db,
            learning.observations,
        )
        .with_supplied_objects(receipt.supplied.iter().map(|c| c.object.clone()).collect())
        .with_recall_receipt(Some(receipt.clone()), Some(override_log.clone()))
        // The session's bound workspace — the only file I/O the read tools
        // reach. `None` (the one-shot `ask` path) leaves them denying with
        // their typed error.
        .with_workspace(session.as_ref().and_then(|session| session.workspace())),
    );
    let request = AgentRequest {
        prompt: prompt.into(),
        profile_names,
        model: ai.model.clone(),
        system_prompt,
        history,
        context_blocks,
    };
    let fallback_approval = TerminalApproval::new(
        approval,
        can_prompt,
        fallback_primary,
        crate::approval_facts::ApprovalFacts::for_ask(runtime),
    );
    let approver: &dyn ApprovalDecider = match decider.as_deref() {
        Some(decider) => decider,
        None => &fallback_approval,
    };
    // Turn and tool-call ceilings come from the environment and are unbounded
    // when unset: SAYA_AGENT_MAX_TURNS / SAYA_AGENT_MAX_TOOL_CALLS, with no
    // upper limit on a set value. The continuation ceiling defaults to its
    // bound when unset (SAYA_AGENT_MAX_CONTINUATIONS), with `0` disabling
    // continuation.
    let env_budgets = saya_agent::budgets_from_env(|name| std::env::var(name).ok());
    // The turn's universe and its executor ride together: a session dispatches
    // through the shared `RunTools` composite and advertises its write-shaped
    // members only where an approval surface exists to answer the asks; the
    // one-shot `ask` path keeps the database surface alone, where
    // `workspace_read` denies with a typed error.
    let definitions = match session.as_ref() {
        Some(session) => session.definitions(
            agent_mode,
            approval,
            can_obtain_approval,
            allow_query_data,
            has_state_store,
            learning.permit_candidate_writes,
        ),
        None => tools::DatabaseTools::definitions(
            allow_query_data,
            has_state_store,
            learning.permit_candidate_writes,
            false,
            // The one-shot `ask` path builds a `TerminalApproval` over this
            // same mode (below) and keeps `permit_external_effects: false`,
            // so the loop's misconfiguration guard stays armed — and a
            // `render_chart` call (`external_side_effect: true`,
            // `requires_approval: true`, no grant token) goes to the
            // decider, which resolves it exactly as a session under the same
            // policy would: per-call ask when an approval surface exists.
            // The chart is advertised exactly there and hidden everywhere
            // else.
            agent_mode == AgentMode::Build
                && match approval {
                    ApprovalPolicy::Ask => can_obtain_approval,
                    ApprovalPolicy::Bypass => true,
                    _ => false,
                },
        ),
    };
    // The fail-closed permits are the definitions' own enforcement, read off
    // them: a turn that advertises a workspace-writing tool must permit
    // workspace writes, or the loop's gate would deny after the ask — and a
    // turn that advertises none keeps the permit off, so nothing can write.
    // Derived, never stated twice, so advertisement and enforcement cannot
    // drift.
    let permit_workspace_writes = definitions
        .iter()
        .any(|definition| definition.effect.local_state == LocalStateEffect::WriteWorkspace);
    let limits = AgentLimits {
        max_turns: env_budgets.max_turns,
        max_tool_calls: env_budgets.max_tool_calls,
        max_continuations: env_budgets.max_continuations,
        permit_candidate_writes: learning.permit_candidate_writes,
        context_byte_budget: runtime.resolved.ai.context_byte_budget,
        permit_workspace_writes,
        // An ask turn approves nothing by scope: the plan-gated egress
        // permit stays off, so a tool with an undeclared approval shape
        // still refuses — the author check, not a user gate.
        permit_external_effects: false,
    };
    let executor: Arc<dyn saya_agent::ToolExecutor> = match session.as_ref() {
        Some(session) => session.executor(Arc::clone(&database), &cancellation),
        None => Arc::clone(&database) as Arc<dyn saya_agent::ToolExecutor>,
    };
    let mut output = run_agent_with_sink(
        &*provider,
        executor.as_ref(),
        request,
        definitions,
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
    // The breaker: session-scoped so it trips across this session's turns; a
    // fresh one per call when there is no session (the one-shot `ask` path,
    // each candidate attempt) — which can never trip within one turn, since
    // there is no next turn for it to disable.
    let fresh_breaker;
    let breaker: &super::learning::LearningBreaker = match session.as_ref() {
        Some(session) => session.learning_breaker(),
        None => {
            fresh_breaker = super::learning::LearningBreaker::new();
            &fresh_breaker
        }
    };
    // The usage the extraction call reported, if any. Stays `None` when
    // learning is disabled, the gate declined, the breaker has already
    // tripped this session, or the call produced no response — absent is
    // not zero.
    let learning_usage = super::learning::post_turn::run_post_turn_extraction(
        super::learning::post_turn::PostTurnInputs {
            permit_candidate_writes: learning.permit_candidate_writes,
            database: &database,
            output: output.as_ref(),
            prompt,
            observations_log: observations_log.as_deref(),
            receipt: &receipt,
            overridden: &overridden,
            provider: &*provider,
            model: &ai.model,
            breaker,
        },
        sink,
    )
    .await;

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
