struct ActionParts<'a> {
    state: &'a mut SessionState,
    runtime: Arc<RuntimeConfig>,
    store: &'a FsSessionStore,
    state_db: &'a SqliteStateStore,
    format: RenderFormat,
    terminal: bool,
    session: &'a mut SessionRuntime,
    save_refused: &'a mut bool,
}

fn action_parts<'a>(ctx: &'a mut TurnContext<'_>) -> ActionParts<'a> {
    ActionParts {
        state: &mut *ctx.state,
        runtime: Arc::clone(&ctx.runtime),
        store: ctx.store,
        state_db: ctx.state_db,
        format: ctx.format,
        terminal: ctx.terminal,
        session: &mut *ctx.session,
        save_refused: &mut ctx.save_refused,
    }
}
