/// The one per-turn entry every headless path shares: the piped loop
/// pre-trims each line (`handle_line`), while `--turn-file` passes the file
/// bytes verbatim (no trimming, no blank-line skipping). An empty turn here
/// is an errored turn — the stream names it and the caller exits non-zero —
/// never silent success.
fn handle_line_verbatim(
    line: &str,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let state = &mut *ctx.state;
    let runtime_handle = Arc::clone(&ctx.runtime);
    let runtime: &RuntimeConfig = &runtime_handle;
    let store = ctx.store;
    let format = ctx.format;
    let session = &mut *ctx.session;
    let save_refused = &mut ctx.save_refused;
    if line.trim().is_empty() {
        // Piped input treats blank lines as no-ops. `read_turn_file` rejects
        // an all-blank file before this entry is reached.
        return Ok(TurnOutcome::Completed);
    }
    // A user-authored command (`<config home>/commands/`, `.saya/commands/`)
    // resolves BEFORE the built-in parser and its expansion is never re-parsed
    // as a slash command: a command file — the project's is untrusted input —
    // can only ever produce a user message, never a config, tool, or
    // permission change. A refused file says why and ends the turn errored.
    let user_prompt = match state.user_commands.route(line) {
        crate::slash::user_commands::UserLine::Builtin => None,
        crate::slash::user_commands::UserLine::Prompt(prompt) => Some(prompt),
        crate::slash::user_commands::UserLine::Refused(message) => {
            session_emit::emit_action(SessionAction::Error(message), format, state, store)?;
            if let Some(outcome) = persist(state, store, format, save_refused)? {
                return Ok(outcome);
            }
            return Ok(TurnOutcome::Errored);
        }
    };
    // A malformed or unknown slash command must not tear down the whole session:
    // surface the parse error (which may carry a "did you mean" hint) and keep looping.
    let parsed = match user_prompt {
        Some(_) => None,
        None => match parse_slash_command(line) {
            Ok(parsed) => parsed,
            Err(error) => {
                session_emit::emit_action(
                    SessionAction::Error(error.to_string()),
                    format,
                    state,
                    store,
                )?;
                if let Some(outcome) = persist(state, store, format, save_refused)? {
                    return Ok(outcome);
                }
                return Ok(TurnOutcome::Errored);
            }
        },
    };
    let action = match parsed {
        Some(command) => {
            // The answering context before and after the command decides
            // whether this command changed it: a changed key retires the
            // question authority once the command's message is produced, an
            // unchanged key leaves a pending clarification untouched.
            let before = state.context_key();
            let action = state.apply(
                command,
                &runtime
                    .connections
                    .profiles
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            state.abandon_question_when_context_changed(&before, session);
            action
        }
        None => {
            // The user message the turn carries is a turn of the session's run:
            // a user command's expansion, or the typed line itself. The
            // expansion is what the run sees — the typed `/name …` is only how
            // it was asked for. Everything the turn prints (the streamed answer,
            // its tool steps, the settled wording) is the line front's; this
            // entry keeps the loop's own bookkeeping.
            let line = user_prompt.as_deref().unwrap_or(line).to_owned();
            return handle_message(&line, ctx);
        }
    };
    if matches!(action, SessionAction::Exit) {
        return Ok(TurnOutcome::Exit);
    }
    finish_action(action, ctx)
}
