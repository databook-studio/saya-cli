fn handle_usage(
    request: crate::slash::UsageRequest,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // `/usage` — one bounded panel of what this session spent and how much of
    // the model's window its last request used, plus the ceiling the request
    // states. Every figure is one the session already holds: no provider call,
    // no database query, no network.
    session_emit::emit_action(
        SessionAction::Message(crate::chat::usage_answer(state, runtime, request)),
        format,
        state,
        store,
    )?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(TurnOutcome::Completed)
}
