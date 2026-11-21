fn handle_status(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        session,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // `/status` — one panel, five sections: the session's identity and record,
    // the model in force with this session's token totals, the workspace
    // binding, the instruction sources the next request would load, and the
    // ceilings in force. Every fact is one the session already holds or one
    // `chat::ops_status` reads without a probe — no provider call, no database
    // query, no network. `/doctor` stays the command that checks.
    session_emit::emit_action(
        SessionAction::Message(crate::chat::ops_status::panel_text(
            state,
            runtime,
            store,
            session.root(),
        )),
        format,
        state,
        store,
    )?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(TurnOutcome::Completed)
}
