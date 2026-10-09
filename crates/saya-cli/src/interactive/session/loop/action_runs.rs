fn handle_runs(
    run_id: Option<String>,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        state_db,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // `/runs [id]` reaches the same `reads.rs` path the headless
    // `saya run list|show` commands use — the parity contract
    // (tests/run_slash_parity.rs) pins the two adapters byte-identical.
    let command = match run_id {
        Some(run_id) => crate::cli::RunCommand::Show { run_id },
        None => crate::cli::RunCommand::List,
    };
    let approval = state
        .approval_mode
        .parse()
        .map_err(|error: saya_agent::ApprovalPolicyParseError| error.to_string())?;
    let code = block_on(crate::commands::run_management(
        command, runtime, format, approval, state_db,
    ))?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(if code == 0 {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    })
}

fn handle_run_cancel(
    run_id: String,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        state_db,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // `/run cancel <id>` is the same engine path `saya run cancel` uses —
    // the shared dispatcher, not a second cancellation implementation.
    let approval = state
        .approval_mode
        .parse()
        .map_err(|error: saya_agent::ApprovalPolicyParseError| error.to_string())?;
    let code = block_on(crate::commands::run_management(
        crate::cli::RunCommand::Cancel { run_id },
        runtime,
        format,
        approval,
        state_db,
    ))?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(if code == 0 {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    })
}
