fn finish_action(
    action: SessionAction,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    if let SessionAction::Coding(command) = action {
        return handle_coding(command, ctx);
    }
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
    if let SessionAction::Mode(next) = action {
        return handle_mode(next, ctx);
    }
    if let SessionAction::Allow(tokens) = action {
        return handle_allow(tokens, ctx);
    }
    if let SessionAction::Permissions(request) = action {
        return handle_permissions(request, ctx);
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
    if let SessionAction::Rotate(request) = action {
        return handle_rotate(request, ctx);
    }
    if let SessionAction::Status = action {
        return handle_status(ctx);
    }
    if let SessionAction::Usage(request) = action {
        return handle_usage(request, ctx);
    }
    if let SessionAction::Reload = action {
        return handle_reload(ctx);
    }
    let ActionParts {
        state,
        store,
        format,
        save_refused,
        ..
    } = action_parts(ctx);
    let outcome = action_outcome(&action);
    session_emit::emit_action(action, format, state, store)?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}
