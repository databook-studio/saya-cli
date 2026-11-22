fn handle_coding(
    request: crate::slash::JobRequest,
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
    // The deprecated spelling's one-release note, the first time this session
    // uses it (`chat::ops_job`): said before the job runs, never instead of it.
    let note = request.note();
    let command = request.command.with_session_write_scope();
    if let Some(note) = note {
        session_emit::emit_action(SessionAction::Message(note.to_owned()), format, state, store)?;
    }
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
        crate::commands::coding::Launch::Attached(session.policy().mode()),
    ))?;
    Ok(if code == 0 {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    })
}
