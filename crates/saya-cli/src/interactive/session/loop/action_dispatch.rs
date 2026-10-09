fn finish_action(
    action: SessionAction,
    before_mode: String,
    approvals_set: bool,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    if let SessionAction::Setup = action {
        return handle_setup(ctx);
    }
    if let SessionAction::Schema(refresh) = action {
        return handle_schema(refresh, ctx);
    }
    if let SessionAction::Doctor = action {
        return handle_doctor(ctx);
    }
    if let SessionAction::Sql(sql) = action {
        return handle_sql(sql, ctx);
    }
    if let SessionAction::Contracts(command) = action {
        return handle_contracts(command, ctx);
    }
    if let SessionAction::Investigation(command) = action {
        return handle_investigation(command, ctx);
    }
    if let SessionAction::Runs(run_id) = action {
        return handle_runs(run_id, ctx);
    }
    if let SessionAction::RunCancel(run_id) = action {
        return handle_run_cancel(run_id, ctx);
    }
    if let SessionAction::Allow(tokens) = action {
        return handle_allow(tokens, ctx);
    }
    if let SessionAction::Grants = action {
        return handle_grants(ctx);
    }
    if let SessionAction::Context = action {
        return handle_context(ctx);
    }
    if let SessionAction::SessionsList = action {
        return handle_sessions_list(ctx);
    }
    if let SessionAction::SessionsDelete(id) = action {
        return handle_sessions_delete(id, ctx);
    }
    if let SessionAction::Preferences(command) = action {
        return handle_preferences(command, ctx);
    }
    if let SessionAction::Recovery(command) = action {
        return handle_recovery(command, ctx);
    }
    if let SessionAction::Run(tail) = action {
        return handle_run(tail, ctx);
    }
    if let SessionAction::Compact = action {
        return handle_compact(ctx);
    }
    if let SessionAction::Resume(id) = action {
        return handle_resume(id, ctx);
    }
    if let SessionAction::Rotate = action {
        return handle_rotate(ctx);
    }
    if let SessionAction::Reload = action {
        return handle_reload(ctx);
    }
    let ActionParts {
        state,
        runtime,
        store,
        format,
        session,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    let outcome = action_outcome(&action);
    session_emit::emit_action(action, format, state, store)?;
    if approvals_set {
        if let Some(line) = crate::interactive::session::permissions::activation::line_if_bypass(
            state,
            runtime,
            &session.universe(),
        ) {
            session_emit::emit_action(SessionAction::Message(line), format, state, store)?;
        }
        if crate::interactive::session::permissions::activation::bypass_activated_by_command(
            &before_mode,
            &state.approval_mode,
        ) && let Err(error) = session
            .journal()
            .bypass_activated(saya_store::BypassSource::Command)
        {
            session_emit::emit_action(
                SessionAction::Message(
                    crate::interactive::session::permissions::grants::journal_warning(&error),
                ),
                format,
                state,
                store,
            )?;
        }
    }
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}
