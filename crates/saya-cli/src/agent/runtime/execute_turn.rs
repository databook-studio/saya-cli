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
    let fallback_approval = Arc::new(
        TerminalApproval::with_cancellation(
            approval,
            can_prompt,
            fallback_primary,
            crate::approval_facts::ApprovalFacts::for_ask(runtime),
            cancellation.clone(),
        )
        .with_metrics(question_scope.and_then(|scope| scope.metrics())),
    );
    // The session's `shell` for this turn: composed per turn, because its mode
    // and its cancellation are this turn's and its operator is this turn's
    // decider — but only where this surface can obtain an approval at all. A
    // headless surface passes no operator, so a host call falls back to the
    // sandbox when one is proven and is refused with a reason when none is,
    // exactly as a run's headless shell decides it. The sandbox underneath was
    // proven once per session (`SessionUniverse::shell`).
    let shell = session.and_then(|session| {
        // A closed plan posture advertises no shell; composing one would leave
        // a door the tool list does not show — and a background start would
        // outlive the turn that made it.
        if agent_mode == AgentMode::Plan && !session.plan_gate_open() {
            return None;
        }
        let operator = can_obtain_approval
            .then(|| decider.map(Arc::clone))
            .flatten();
        session.shell(approval, operator, cancellation.clone())
    });
    // The session's MCP servers for this turn: the servers started once at
    // acquisition, under this turn's mode, this turn's operator, and this
    // turn's cancellation. A turn with no approval surface passes no
    // operator, and a call is then refused with the reason naming the flag
    // that would reach a server.
    let mcp = match session {
        Some(session) => {
            let consent = match can_obtain_approval.then(|| decider.map(Arc::clone)).flatten() {
                Some(decider) => crate::mcp::client::Consent::Decider(decider),
                None => crate::mcp::client::Consent::Absent,
            };
            session.mcp_for_turn(approval, consent, cancellation)
        }
        // The one-shot `ask` surface has no session to hold servers between
        // turns, and no operator to answer a call: the lane is composed for
        // this turn and dropped with it, and every call is refused with the
        // typed reason naming the flag that would reach a server.
        None => crate::agent::runtime::ask_surface::lane(runtime, approval, cancellation).await,
    };
    // A `shell` call is decided by the shell's own lane decision, never by the
    // session's policy engine: the sandbox lane because the confinement is the
    // consent, and the host lane because the mode (or, under `ask`, the
    // operator) already answered. An MCP call is decided the same way, by its
    // own lane: a server is a program whose effects this mode cannot bound.
    // Every other call keeps the wrapped decider's answer.
    // The user's own hooks, in front of that executor and nothing else: the
    // gate can refuse a call the surface allowed, and can never let one
    // through the surface would have refused. A session's hooks run over the
    // session's own root; the one-shot `ask` path has no root of its own and
    // runs them over the directory it was started in.
    let hook_root = session
        .and_then(|session| session.root().map(std::path::Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok());
    let hooks = hook_root.as_deref().and_then(|root| {
        let caller = match session {
            Some(session) => HookCaller::Session(session.session_id()),
            None => HookCaller::Ask,
        };
        Hooks::compose(runtime, approval, root, caller, cancellation.clone())
    });
    // The `SessionStart` context runs once per session process — at launch,
    // or here where the launch had no root yet (a trust answer binds later).
    // Every later turn reads the same bounded text.
    let session_context = match (session, hooks.as_ref()) {
        (Some(session), Some(hooks)) => session.session_hooks().start(hooks).await,
        _ => None,
    };
    let fallback: Arc<dyn ApprovalDecider> = fallback_approval;
    let inner: Arc<dyn ApprovalDecider> = match decider {
        Some(inner) => Arc::clone(inner),
        None => fallback,
    };
    let owners: Vec<Arc<dyn OwnApproval>> = [shell
        .as_ref()
        .map(|shell| Arc::clone(shell) as Arc<dyn OwnApproval>)]
    .into_iter()
    .chain([mcp
        .as_ref()
        .map(|mcp| Arc::clone(mcp) as Arc<dyn OwnApproval>)])
    .flatten()
    .collect();
    let base: Arc<dyn ApprovalDecider> = if owners.is_empty() {
        inner
    } else {
        Arc::new(OwnedToolApproval::new(owners, inner))
    };
    // A `PermissionRequest` hook runs only where the operator was about to be
    // asked, and can only refuse that ask: exit `0` falls through to the same
    // ask the surface would have made.
    let approval_gate;
    let approver: &dyn ApprovalDecider = match hooks.as_ref() {
        Some(hooks) => {
            approval_gate = HookApprovalGate::new(Arc::clone(hooks), base);
            &approval_gate
        }
        None => base.as_ref(),
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
    // `read` denies with a typed error.
    // The one `[tools] allow` list is applied *inside* each composition, after
    // it has assembled its surface, so it can only ever take a tool away. It
    // runs before the permits are derived from the definitions below, so a
    // surface narrowed out of its write tools also loses the write permit.
    // The session's plan gate asks through this turn's own approval surface —
    // the same modal (TUI) or terminal prompt (REPL) every other ask of the
    // turn goes through — bound per turn, exactly as the shell's and the MCP
    // lane's operators are. A turn with no decider binds nothing, and a
    // submitted plan is then refused: no reader, no approval.
    if let Some(session) = session {
        session.bind_plan_ask(decider.map(Arc::clone));
    }
    let definitions = match session {
        Some(session) => session.definitions(
            agent_mode,
            session.plan_gate_open(),
            approval,
            can_obtain_approval,
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
            &runtime.resolved.tools,
        ),
        // The one-shot `ask` path composes the database surface and, where a
        // lane exists, the configured servers' tools: no workspace writes, so
        // `permit_workspace_writes` stays off.
        None => crate::agent::runtime::ask_surface::definitions(
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
            mcp.as_ref(),
            &runtime.resolved.tools,
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
        // An interactive answering turn keeps its current limits; progress
        // guards belong to jobs and run steps.
        progress: None,
    };
    let executor: Arc<dyn saya_agent::ToolExecutor> = match session {
        Some(session) => session.executor(
            Arc::clone(database),
            cancellation,
            shell.clone(),
            mcp.clone(),
            // The gate rides the turn exactly where `submit_plan` is
            // advertised — under `plan` — so the door is never wider than the
            // list the model was offered.
            (agent_mode == AgentMode::Plan).then(|| session.plan_gate()),
            runtime.resolved.tools.clone(),
        ),
        // The ask surface's own composition is `DatabaseTools` plus its MCP
        // lane; the allow list rides a thin gate in front of both, so a name
        // the list removed is refused at the door and not merely
        // unadvertised.
        None => crate::agent::runtime::ask_surface::executor(
            database,
            mcp.clone(),
            runtime.resolved.tools.clone(),
        ),
    };
    let executor: Arc<dyn saya_agent::ToolExecutor> = match hooks.as_ref() {
        Some(hooks) => Arc::new(
            HookGate::new(Arc::clone(hooks), executor).with_session_context(session_context),
        ),
        None => executor,
    };
    // Only reached without a question scope (tests, and any headless caller
    // that composes no question): a scope carries its own surface's ceilings and
    // overrides this one below. The session's are the fallback because a
    // scope-less turn is the session's — the one shared budget meant exactly
    // that before `ask` could state its own.
    let answering = answering_limits(runtime, AnsweringSurface::Session);
    let control = AgentRunControl::new(limits, answering, approver, sink, cancellation.clone());
    let control = match question_scope.as_ref() {
        Some(scope) => control.with_answering_scope(scope.answering_scope()),
        None => control,
    }
    .with_request_preflight(preflight);
    let output =
        run_agent_with_control(provider, executor.as_ref(), request, definitions, control).await;
    // The turn is over: the `stop` hooks run now, after the loop's own work
    // and whatever its outcome was. They observe the turn's end and decide
    // nothing about it.
    if let Some(hooks) = hooks.as_ref() {
        hooks.stop().await;
    }
    if let Some(scope) = question_scope.as_ref() {
        scope.finish_segment();
    }
    if let Some(answering_settled) = answering_settled {
        let _ = answering_settled.send(());
    }

    output
}
