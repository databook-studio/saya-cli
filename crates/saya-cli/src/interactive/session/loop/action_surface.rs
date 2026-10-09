fn handle_setup(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        store,
        format,
        save_refused,
        ..
    } = action_parts(ctx);
    // The headless REPL has no terminal to hand over: the refusal names
    // the full-screen TUI and the shell command to run instead. An
    // errored turn, so `--turn-file` exits non-zero — the same shape as
    // the TUI-only refusals (`/export`, `/report`).
    session_emit::emit_action(
        SessionAction::Error(
            crate::interactive::session::commands::SETUP_TUI_ONLY_ERROR.to_owned(),
        ),
        format,
        state,
        store,
    )?;
    persist(state, store, format, save_refused)?;
    Ok(TurnOutcome::Errored)
}

fn handle_schema(
    refresh: bool,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        terminal,
        format,
        state_db,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    let completed = block_on(super::super::session_schema::run(
        runtime,
        state.profile.as_deref(),
        refresh,
        terminal,
        format,
        state_db,
    ))?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(if completed {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    })
}

fn handle_doctor(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // Same report the TUI's /doctor shows; runtime lives here.
    session_emit::emit_action(
        SessionAction::Message(crate::config::doctor::summary(runtime)),
        format,
        state,
        store,
    )?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(TurnOutcome::Completed)
}

fn handle_sql(
    sql: String,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        terminal,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    block_on(super::super::session_sql::run(
        runtime,
        state.profile.as_deref(),
        &sql,
        terminal,
        format,
    ))?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(TurnOutcome::Completed)
}

fn handle_contracts(
    command: crate::cli::ContractsCommand,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        format,
        state_db,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // The slash adapter hands the translated `ContractsCommand` to the same
    // `run_contracts` dispatcher the headless `saya contracts` path uses; the
    // captured output goes to the terminal through the shared `emit` seam.
    let code = block_on(crate::commands::run_contracts(
        command, runtime, format, state_db,
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

fn handle_investigation(
    command: crate::cli::InvestigationCommand,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        terminal,
        format,
        state_db,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // The headless REPL has no last query to save from (invariant 3): a
    // save with neither `--sql` nor `--file` would fall through to the
    // shared operation's stdin read, and stdin here is the REPL's own
    // input — so the save is refused before the operation runs. Every
    // other subcommand goes to the same `run_investigation` dispatcher
    // the clap path uses, captured output and all; `terminal` is the
    // can-prompt answer the `/sql` arm passes.
    if let crate::cli::InvestigationCommand::Save {
        sql: None,
        file: None,
        ..
    } = &command
    {
        session_emit::emit_action(
            SessionAction::Error(
                "investigation save needs the SQL here: pass --sql <SQL> or \
                 --file <PATH>"
                    .into(),
            ),
            format,
            state,
            store,
        )?;
        if let Some(outcome) = persist(state, store, format, save_refused)? {
            return Ok(outcome);
        }
        return Ok(TurnOutcome::Errored);
    }
    let code = block_on(crate::commands::run_investigation(
        command, runtime, format, terminal, state_db,
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
