fn handle_resume(
    id: String,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        session,
        store,
        format,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    let defaults = crate::interactive::session::lifecycle::resume::SessionDefaults {
        provider: state.provider.clone(),
        model: state.model.clone(),
        allow_data_sharing: state.allow_data_sharing,
        approval_mode: state.approval_mode.clone(),
    };
    let mut outcome = TurnOutcome::Errored;
    match crate::interactive::session::lifecycle::resume::resume_session(store, &id, &defaults) {
        Ok(Some(loaded)) => {
            // The resumed session's own state must ride the swap: the new
            // state directory claimed and composed before the old lock
            // releases; a refused swap keeps this session. The resumed
            // session's policy is its own — grants are process-lifetime
            // facts about one session, and a resumed session starts empty.
            match session.reacquire(
                runtime,
                loaded.workspace_root.as_deref(),
                &id,
                loaded
                    .approval_mode
                    .parse()
                    .unwrap_or(saya_agent::ApprovalPolicy::Ask),
            ) {
                Ok(()) => {
                    outcome = TurnOutcome::Completed;
                    *state = loaded;
                    state.bind_runtime_endpoint(runtime.resolved.ai.base_url.as_deref());
                    session.universe().seed_tasks(state.task_list.clone());
                    // The header facts ride the NEW universe: `reacquire`
                    // recomposed it, and the swapped-in state arrived with
                    // the `#[serde(skip)]` defaults.
                    state.sync_host_posture(&session.universe());
                    session_emit::emit_action(
                        SessionAction::Message(format!("Resumed session {id}")),
                        format,
                        state,
                        store,
                    )?;
                    if let Some(notice) = session.notice() {
                        session_emit::emit_action(
                            SessionAction::Message(notice.to_string()),
                            format,
                            state,
                            store,
                        )?;
                    }
                    // A resumed bypass session re-prints its activation
                    // line — the mode is real again, and the user reads
                    // its wording once, not a bare `approval:bypass` on
                    // the status bar.
                    if let Some(line) =
                        crate::interactive::session::permissions::activation::line_if_bypass(
                            state,
                            runtime,
                            &session.universe(),
                        )
                    {
                        session_emit::emit_action(
                            SessionAction::Message(line),
                            format,
                            state,
                            store,
                        )?;
                    }
                }
                Err(error) => {
                    session_emit::emit_action(SessionAction::Error(error), format, state, store)?
                }
            }
        }
        Ok(None) => session_emit::emit_action(
            SessionAction::Error(format!("Session not found: {id}")),
            format,
            state,
            store,
        )?,
        Err(error) => session_emit::emit_action(
            SessionAction::Error(error.to_string()),
            format,
            state,
            store,
        )?,
    }
    // The resumed session's save: a near-limit notice fires here for a
    // large record the resume just re-opened — the per-process flag is
    // armed again on the fresh state.
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}

fn handle_rotate(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        session,
        store,
        format,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // `/rotate` — continue in a small successor. The shared steps run
    // once, in `session_rotate_run`, for both surfaces: save the
    // parent, publish the lineage-linked successor, re-acquire the
    // runtime for it, and swap only on success. A refusal or a failed
    // swap is an errored turn; the completed rotation's save below
    // persists the successor (the swapped state).
    let outcome = match crate::interactive::session::lifecycle::rotate_run::rotate_session(
        store, runtime, state, session,
    ) {
        crate::interactive::session::lifecycle::rotate_run::RotateResult::Rotated { old, new } => {
            session_emit::emit_action(
                SessionAction::Message(
                    crate::interactive::session::lifecycle::rotate_run::rotated_message(&old, &new),
                ),
                format,
                state,
                store,
            )?;
            if let Some(notice) = session.notice() {
                session_emit::emit_action(
                    SessionAction::Message(notice.to_string()),
                    format,
                    state,
                    store,
                )?;
            }
            // A bypass successor re-prints its activation line — the
            // mode is real again in the new session, the same tail a
            // resumed bypass session gets.
            if let Some(line) = crate::interactive::session::permissions::activation::line_if_bypass(
                state,
                runtime,
                &session.universe(),
            ) {
                session_emit::emit_action(SessionAction::Message(line), format, state, store)?;
            }
            TurnOutcome::Completed
        }
        crate::interactive::session::lifecycle::rotate_run::RotateResult::Refused(message)
        | crate::interactive::session::lifecycle::rotate_run::RotateResult::Incomplete(message) => {
            session_emit::emit_action(SessionAction::Error(message), format, state, store)?;
            TurnOutcome::Errored
        }
    };
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}

fn handle_reload(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let TurnContext {
        state,
        runtime: runtime_handle,
        session,
        store,
        format,
        save_refused,
        launch_options,
        ..
    } = ctx;
    let runtime = Arc::clone(runtime_handle);
    let runtime: &RuntimeConfig = &runtime;
    // `/reload` re-reads the configuration with the SAME launch options
    // and, only on success, swaps the loop's runtime handle — the swap
    // is observed by the NEXT line, never mid-turn. The shared operation
    // plans every change against the running runtime, re-seeds the
    // session copies the user never overrode, and renders the report;
    // data sharing, host commands, grants and the approval mode are
    // classified pending and never move. A failed build changes nothing
    // and says the previous configuration is still in use. Never
    // prompts: the inputs are the launch's, and the pipe stays the pipe.
    let inputs = crate::config::reload::ReloadInputs::from_launch(launch_options);
    // The re-seed runs after the `state.apply` comparison above, so the
    // reload captures the key here and retires the question authority
    // once a successful reload moved it — the same shared abandon step
    // every mutating command uses: a pending clarification must not
    // answer under the new provider/model/profile with the old
    // question's scope.
    let before = state.context_key();
    match crate::config::reload::reload_runtime(&inputs, runtime, state) {
        crate::config::reload::ReloadOutcome::Reloaded { new, text } => {
            *runtime_handle = Arc::new(*new);
            session_emit::emit_action(SessionAction::Message(text), *format, state, store)?;
            state.abandon_question_when_context_changed(&before, session);
            if let Some(outcome) = persist(state, store, *format, save_refused)? {
                return Ok(outcome);
            }
            Ok(TurnOutcome::Completed)
        }
        crate::config::reload::ReloadOutcome::Failed(text) => {
            session_emit::emit_action(SessionAction::Error(text), *format, state, store)?;
            if let Some(outcome) = persist(state, store, *format, save_refused)? {
                return Ok(outcome);
            }
            Ok(TurnOutcome::Errored)
        }
    }
}
