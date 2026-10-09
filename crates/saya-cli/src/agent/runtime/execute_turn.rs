// These are independent turn policy and execution inputs; bundling them would
// hide the call site's explicit approval, session, and runtime decisions.
#[allow(clippy::too_many_arguments)]
async fn run_agent_turn(
    runtime: &RuntimeConfig,
    preflight: saya_agent::RequestPreflightConfig,
    approval: ApprovalPolicy,
    can_prompt: bool,
    can_obtain_approval: bool,
    fallback_primary: TurnPrimary,
    cancellation: &CancellationToken,
    question_scope: Option<&QuestionScope>,
    decider: Option<&Arc<dyn ApprovalDecider>>,
    session: Option<&Arc<SessionUniverse>>,
    agent_mode: AgentMode,
    allow_query_data: bool,
    has_state_store: bool,
    permit_candidate_writes: bool,
    database: &Arc<tools::DatabaseTools>,
    request: AgentRequest,
    provider: &dyn saya_agent::ChatProvider,
    sink: &dyn AgentEventSink,
    answering_settled: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<AgentOutput, AgentError> {
    let fallback_approval = TerminalApproval::with_cancellation(
        approval,
        can_prompt,
        fallback_primary,
        crate::approval_facts::ApprovalFacts::for_ask(runtime),
        cancellation.clone(),
    )
    .with_metrics(question_scope.and_then(|scope| scope.metrics()));
    let approver: &dyn ApprovalDecider = match decider.map(Arc::as_ref) {
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
    let definitions = match session {
        Some(session) => session.definitions(
            agent_mode,
            approval,
            can_obtain_approval,
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
        ),
        None => tools::DatabaseTools::definitions(
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
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
        permit_candidate_writes,
        context_byte_budget: runtime.resolved.ai.context_byte_budget,
        permit_workspace_writes,
        // An ask turn approves nothing by scope: the plan-gated egress
        // permit stays off, so a tool with an undeclared approval shape
        // still refuses — the author check, not a user gate.
        permit_external_effects: false,
    };
    let executor: Arc<dyn saya_agent::ToolExecutor> = match session {
        Some(session) => {
            session.executor(Arc::clone(database), cancellation, permit_workspace_writes)
        }
        None => Arc::clone(database) as Arc<dyn saya_agent::ToolExecutor>,
    };
    let answering = AnsweringLimits {
        max_requests: finite_limit(
            runtime
                .resolved
                .investigation_budgets
                .logical_answering_requests,
        ),
        max_tool_calls: finite_limit(runtime.resolved.investigation_budgets.requested_tool_calls),
        max_reported_tokens: finite_limit(
            runtime.resolved.investigation_budgets.known_reported_tokens,
        ),
    };
    let control = AgentRunControl::new(limits, answering, approver, sink, cancellation.clone());
    let control = match question_scope.as_ref() {
        Some(scope) => control.with_answering_scope(scope.answering_scope()),
        None => control,
    }
    .with_request_preflight(preflight);
    let output =
        run_agent_with_control(provider, executor.as_ref(), request, definitions, control).await;
    if let Some(scope) = question_scope.as_ref() {
        scope.finish_segment();
    }
    if let Some(answering_settled) = answering_settled {
        let _ = answering_settled.send(());
    }

    output
}
