fn run_terminal(ctx: LaunchContext) -> Result<i32, Box<dyn std::error::Error>> {
    let LaunchContext {
        cli,
        runtime,
        store,
        state_db,
        format,
        mut state,
        mut session,
        fresh,
        trusted_echo,
        tui_trust_pending,
        launch,
        ..
    } = ctx;
    let trusted_echo_for_tui = trusted_echo.clone();
    // Interactive terminals get the full-screen TUI. Chart temp files are
    // removed when the session ends, on the clean path and on error alike.
    // The startup trust question rides the TUI's modal when it is still
    // open — asked inside the interface after the splash paints, never
    // as a raw stdin read in front of it. The modal binds through the
    // live runtime when answered; the pin and the header facts below
    // refresh from the rebound session when the TUI returns them.
    let outcome = super::super::tui::run(super::super::tui::TuiSession {
        runtime: &runtime,
        store: &store,
        state_db: &state_db,
        format,
        state: &mut state,
        session: &mut session,
        trusted_echo: trusted_echo_for_tui.as_deref(),
        trust_pending: tui_trust_pending,
        launch: &launch,
        // The CLI options ride along so `/setup` can build the wizard's
        // real flow options (`FlowOptions::real`) at invocation time.
        options: &cli.options,
    });
    // The terminal is restored — the TUI's guard dropped when `run`
    // returned — so the last settled question's metrics section can go
    // to stderr now, never while the TUI owned the screen.
    if let Some(snapshot) = session.take_question_metrics() {
        eprintln!(
            "{}",
            crate::local_metrics_format::format_snapshot(&snapshot)
        );
    }
    crate::local_metrics_format::print_metrics_section(&session.session_metrics());
    if let Ok(super::super::tui::TrustOutcome::Answered(dir)) = &outcome {
        let _ = dir;
    }
    crate::chart::cleanup_session_charts();
    let code = outcome.map(|outcome| outcome.exit_code())?;
    if let Some(root) = session.record_root(fresh) {
        state.workspace_root = Some(root);
    }
    state.sync_host_posture(&session.universe());
    // The last save of a TUI session: a refusal at the record cap says
    // its sentence through the Error event once the terminal is
    // restored, and keeps the TUI's own exit code.
    match save_with_explanation(&store, &state) {
        Err(SaveFailure::LimitReached(text)) => {
            session_emit::emit_action(SessionAction::Error(text), format, &mut state, &store)?
        }
        Err(SaveFailure::Other(error)) => return Err(error.into()),
        Ok(()) => {}
    }
    Ok(code)
}
