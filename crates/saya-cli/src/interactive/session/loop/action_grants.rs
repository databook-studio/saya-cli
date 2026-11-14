fn handle_allow(
    tokens: Vec<String>,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
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
    // `/allow <scopes…>` seeds the session's one grant store through the
    // shared behaviour — the same parser, the session surface, and the
    // same composition the prompts state: a token the session composed
    // no capability for is refused there too, never seeded. A refused
    // scope is an error and seeds nothing; `/allow none` seeds nothing
    // and says so. Each newly seeded token is journalled once by the
    // shared behaviour; a failed journal write changes no grant and is
    // said in the message.
    let outcome = match crate::interactive::session::permissions::grants::allow(
        &tokens,
        &session.universe().approval_facts(runtime),
        session.policy().grants(),
        &session.journal(),
    ) {
        Ok(message) => {
            session_emit::emit_action(SessionAction::Message(message), format, state, store)?;
            TurnOutcome::Completed
        }
        Err(error) => {
            session_emit::emit_action(SessionAction::Error(error), format, state, store)?;
            TurnOutcome::Errored
        }
    };
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}

fn handle_grants(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        session,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    // `/grants` lists the store verbatim: the words are the record, and
    // the mode the store sits under is the engine's own — under bypass
    // it is stated first, so the count never reads "nothing runs".
    session_emit::emit_action(
        SessionAction::Message(crate::interactive::session::permissions::grants::listing(
            session.policy().mode(),
            session.policy().grants(),
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

fn handle_context(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
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
    // `/context` shows the instruction sources the next request would
    // load — names, sizes, short digests, truncation and redaction
    // flags, and skip/omit reasons — through the one shared renderer
    // the TUI prints too. Never the file text. The view needs the
    // runtime's settings and the session's workspace root, so it is
    // intercepted here like /grants.
    session_emit::emit_action(
        SessionAction::Message(crate::interactive::session::context_view::context_view(
            runtime,
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
