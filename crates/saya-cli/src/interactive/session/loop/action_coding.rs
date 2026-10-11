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
            SessionAction::Error("Bind a repository workspace before starting coding jobs.".into()),
            format,
            state,
            store,
        )?;
        return Ok(TurnOutcome::Errored);
    };
    let code = block_on(crate::commands::coding::cli::run(
        command,
        root.to_path_buf(),
        runtime
            .resolved
            .jobs
            .runner
            .program_dir
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("/usr/bin")),
        &runtime,
        format,
        state_db,
    ))?;
    Ok(if code == 0 {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    })
}
