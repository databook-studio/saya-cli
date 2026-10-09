pub(crate) async fn run_prompt_with_continuation_inputs_with_signal(
    turn: TurnPrompt<'_>,
    answering_settled: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<AgentOutput, AgentRuntimeError> {
    let TurnPrompt {
        runtime,
        inputs,
        prompt,
        approval,
        can_prompt,
        can_obtain_approval,
        history,
        sink,
        cancellation,
        question_scope,
        state_db,
        decider,
        last_sql,
        session,
        agent_mode,
        capture,
        prior_tool_outcomes,
        continuation,
        compaction_summary,
    } = turn;
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
    let (context_blocks, _task_list) = append_session_context(
        context_blocks,
        session.as_ref(),
        prompt,
        &registry,
        &receipt,
        prior_tool_outcomes.as_ref(),
        continuation.as_ref(),
        compaction_summary.as_deref(),
        ai.context_byte_budget,
    );
    let context_blocks = project_turn_context_blocks(
        runtime,
        session.as_ref(),
        state_db.as_ref(),
        prompt,
        system_prompt.as_deref(),
        context_blocks,
    )
    .await;
    sink.emit(knowledge_supplied_event(&receipt)).await;
    let receipt = Arc::new(receipt);
    let learning = super::learning::LearningSetup::from(memory.mode);
    let observations_log = learning.observations.clone();
    let has_state_store = state_db.is_some();
    let override_log = Arc::new(tools::OverrideLog::new());
    let database = Arc::new(super::session_recovery_attach::attach_recovery(
        tools::DatabaseTools::with_learning(
            registry,
            runtime.resolved.max_rows,
            allow_query_data,
            state_db,
            learning.observations,
        )
        .with_metrics(question_scope.as_ref().and_then(|scope| scope.metrics()))
        .with_supplied_objects(receipt.supplied.iter().map(|c| c.object.clone()).collect())
        .with_recall_receipt(Some(receipt.clone()), Some(override_log.clone()))
        // The session's bound workspace — the only file I/O the read tools
        // reach. `None` (the one-shot `ask` path) leaves them denying with
        // their typed error.
        .with_workspace(session.as_ref().and_then(|session| session.workspace()))
        // The TUI's capture hook (C1) together with the turn's context byte budget
        // — the SAME value `AgentLimits::context_byte_budget` below carries,
        // so the capture hook shapes the model's view with the budget the
        // loop shapes it with (R3). `None` headless.
        .with_capture(capture, runtime.resolved.ai.context_byte_budget),
        // The session's recovery recorder — no session, nothing attached.
        session.as_deref(),
    ));
    let request = AgentRequest {
        prompt: prompt.into(),
        profile_names,
        model: ai.model.clone(),
        system_prompt,
        history,
        context_blocks,
    };
    let preflight = super::preflight_config::request_preflight_config(&ai)?;
    let output = run_agent_turn(
        runtime,
        preflight,
        approval,
        can_prompt,
        can_obtain_approval,
        fallback_primary,
        &cancellation,
        question_scope.as_ref(),
        decider.as_ref(),
        session.as_ref(),
        agent_mode,
        allow_query_data,
        has_state_store,
        learning.permit_candidate_writes,
        &database,
        request,
        provider.as_ref(),
        sink,
        answering_settled,
    )
    .await;
    finish_turn(
        output,
        override_log,
        sink,
        &cancellation,
        session.as_ref(),
        learning.permit_candidate_writes,
        &database,
        prompt,
        observations_log.as_deref(),
        &receipt,
        provider.as_ref(),
        &ai.model,
    )
    .await
}
