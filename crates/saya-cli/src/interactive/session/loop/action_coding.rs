fn handle_coding(
    command: crate::cli::CodingCommand,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        session,
        format,
        store,
        state_db,
        ..
    } = action_parts(ctx);
    let Some(root) = session.root() else {
        session_emit::emit_action(
            SessionAction::Error("Bind a repository workspace before starting a job.".into()),
            format,
            state,
            store,
        )?;
        return Ok(TurnOutcome::Errored);
    };
    // A session start with no `--write` edits the whole workspace, like
    // `saya -p`; an explicit `--write` narrows. The shared `dispatch` below
    // is also the headless `saya start` path, so the default is applied here,
    // on the session surface, and never there.
    let command = command.with_session_write_scope();
    let code = block_on(crate::commands::coding::cli::run(
        command,
        root.to_path_buf(),
        runtime
            .resolved
            .coding
            .program_dir
            .clone()
            .unwrap_or_else(crate::cli::default_program_dir),
        &runtime,
        format,
        state_db,
        session.policy().mode(),
    ))?;
    Ok(if code == 0 {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    })
}
