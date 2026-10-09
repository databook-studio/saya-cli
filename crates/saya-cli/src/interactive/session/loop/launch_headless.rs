fn run_headless(ctx: LaunchContext) -> Result<i32, Box<dyn std::error::Error>> {
    let LaunchContext {
        cli,
        runtime,
        store,
        state_db,
        format,
        mut state,
        mut session,
        trusted_echo,
        terminal_probe,
        ..
    } = ctx;
    let runtime = Arc::new(runtime);
    let terminal = terminal_probe;
    // Piped / non-TTY input (scripts, CI) uses the headless line executor.
    // `--turn-file` runs one verbatim turn instead: the file bytes reach the
    // same per-turn entry (`handle_line`) with no line splitting, no
    // trimming, and no blank-line skipping, then the process exits.
    if let Some(path) = &cli.options.turn_file {
        let turn = match read_turn_file(path) {
            Ok(turn) => turn,
            Err(error) => {
                let rendered = crate::render_event(
                    &crate::TerminalEvent::Error {
                        message: error.clone(),
                    },
                    format,
                );
                print!("{}", rendered.stdout);
                eprint!("{}", rendered.stderr);
                // The run failed before its turn: the promised terminal
                // result still ends stdout, with the exit code the process
                // returns for the propagated error (2).
                emit_turn_result(format, TurnStatus::Failed, 2, ToolTally::default());
                return Err(error.into());
            }
        };
        let mut ctx = TurnContext {
            state: &mut state,
            runtime: Arc::clone(&runtime),
            store: &store,
            state_db: &state_db,
            format,
            terminal: false,
            session: &mut session,
            trusted_echo: None,
            last_turn: None,
            save_refused: false,
            launch_options: cli.options.clone(),
        };
        let outcome = run_single_turn(turn, &mut ctx);
        crate::chart::cleanup_session_charts();
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                // The run failed during its turn (a store or IO error, a
                // bad persisted approval mode): the terminal result still
                // ends stdout with the process's exit code, whatever tool
                // tally the turn left behind, and the error propagates.
                let tally = ctx
                    .last_turn
                    .take()
                    .map(|summary| summary.tally)
                    .unwrap_or_default();
                emit_turn_result(ctx.format, TurnStatus::Failed, 2, tally);
                return Err(error);
            }
        };
        // The turn's settled summary and the save-refusal fact, taken before
        // the save reborrows the state: the render happens after the final
        // save so the reported exit code is the code the process returns.
        let summary = ctx.last_turn.take();
        let save_refused = ctx.save_refused;
        // The scripted run's final save: a refusal at the record cap makes
        // the run errored even when the turn itself completed, and — when
        // the per-turn save already said the refusal — never repeats the
        // identical sentence. The last good record stays on disk, resumable
        // by id. Any other store error propagates after its own failed-run
        // result.
        let save_errored = match save_with_explanation(&store, &state) {
            Err(SaveFailure::LimitReached(text)) => {
                if !save_refused {
                    session_emit::emit_action(
                        SessionAction::Error(text),
                        format,
                        &mut state,
                        &store,
                    )?;
                }
                true
            }
            Err(SaveFailure::Other(error)) => {
                let tally = summary
                    .as_ref()
                    .map(|turn| turn.tally.clone())
                    .unwrap_or_default();
                emit_turn_result(format, TurnStatus::Failed, 2, tally);
                return Err(error.into());
            }
            Ok(()) => false,
        };
        // The final outcome: the turn's settled status corrected by the
        // run's own outcome — a completed turn whose save was refused ends
        // errored — and the exit code computed once from it, rendered and
        // returned as the same value.
        let run_errored = matches!(outcome, TurnOutcome::Errored) || save_errored;
        let base = summary.unwrap_or_else(|| TurnSummary {
            status: if run_errored {
                TurnStatus::Failed
            } else {
                TurnStatus::Completed
            },
            tally: ToolTally::default(),
        });
        let settled = super::super::turn_summary::final_summary(base, run_errored);
        let code = if run_errored { 5 } else { 0 };
        emit_turn_result(format, settled.status, code, settled.tally);
        return Ok(code);
    }
    let mut ctx = TurnContext {
        state: &mut state,
        runtime: Arc::clone(&runtime),
        store: &store,
        state_db: &state_db,
        format,
        terminal,
        session: &mut session,
        trusted_echo,
        last_turn: None,
        save_refused: false,
        launch_options: cli.options.clone(),
    };
    let outcome = run_plain_loop(&mut ctx);
    crate::chart::cleanup_session_charts();
    outcome?;
    // The piped loop's last save: a refusal at the record cap says its
    // sentence through the Error event (the loop has ended; nothing more to
    // continue) and keeps today's exit contract.
    match save_with_explanation(&store, &state) {
        Err(SaveFailure::LimitReached(text)) => {
            session_emit::emit_action(SessionAction::Error(text), format, &mut state, &store)?
        }
        Err(SaveFailure::Other(error)) => return Err(error.into()),
        Ok(()) => {}
    }
    Ok(0)
}
