fn handle_sessions_list(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        store,
        format,
        save_refused,
        ..
    } = action_parts(ctx);
    // `/sessions list` — the same rows, pending deletions, and storage
    // root the headless `saya sessions list` prints, through the same
    // shared operations; intercepted here like /grants (it needs the
    // store and the sessions root, both built from one dir).
    let (outcome, emitted) = match crate::interactive::session::commands::sessions_list_text(
        store,
        &default_session_dir(),
    ) {
        Ok(text) => (TurnOutcome::Completed, SessionAction::Message(text)),
        Err(text) => (TurnOutcome::Errored, SessionAction::Error(text)),
    };
    session_emit::emit_action(emitted, format, state, store)?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}

fn handle_sessions_delete(
    id: String,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        store,
        format,
        save_refused,
        ..
    } = action_parts(ctx);
    // `/sessions delete <id>` — the same shared operation the headless
    // `saya sessions delete` runs, never prompting. This session's own
    // id is passed as the current one, so a session can never delete
    // itself underneath its owner; every refusal is an errored turn,
    // exactly like the generic Error path, so `--turn-file` exits
    // non-zero.
    let (ok, message) = crate::interactive::session::commands::sessions_delete_text(
        store,
        &default_session_dir(),
        &id,
        Some(&state.id),
    );
    let outcome = if ok {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    };
    let emitted = if ok {
        SessionAction::Message(message)
    } else {
        SessionAction::Error(message)
    };
    session_emit::emit_action(emitted, format, state, store)?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}
