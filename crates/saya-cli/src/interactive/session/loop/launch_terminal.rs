fn run_terminal(ctx: LaunchContext) -> Result<i32, Box<dyn std::error::Error>> {
    let LaunchContext {
        runtime,
        store,
        state_db,
        format,
        state,
        session,
        fresh,
        tui_trust_pending,
        launch,
        cli,
        ..
    } = ctx;
    let entered = std::time::Instant::now();
    // The startup trust question rides the surface's own pane (`02` §9.4): the
    // session an answer binds through is handed to the store, and taken back
    // when the surface ends. A raw stdin read in front of the interface would
    // pre-empt the alternate screen and leave the terminal broken.
    let runtime = std::sync::Arc::new(runtime);
    // The session's facts, and the record this launch resumed: the one opener
    // reads its link exactly as the plain loop's first message does, so an
    // attach, a next epoch, a read-only record and a refusal all open the same
    // way on both surfaces.
    let composed = surface_parts(
        &state,
        &session,
        runtime.as_ref(),
        &state_db,
        format,
    );
    let model = surface_model(&state, fresh);
    // `NO_COLOR` is read here, once, and handed to the pure resolver: any
    // non-empty value selects the no-colour column (02 §12 rule 4, no-color.org),
    // and nothing later in the process reads it again.
    let colour_env: std::collections::BTreeMap<String, String> =
        std::env::var(crate::front::tui::theme::NO_COLOR)
            .ok()
            .map(|value| {
                [(
                    String::from(crate::front::tui::theme::NO_COLOR),
                    value,
                )]
                .into_iter()
                .collect()
            })
            .unwrap_or_default();
    let facts = surface_facts(
        &state,
        tui_trust_pending,
        crate::front::tui::theme::scheme(
            &colour_env,
            crate::front::tui::theme::Scheme::Dark,
        ),
    );
    let commands = state.user_commands.clone();
    let metrics = session.session_metrics();
    // The configuration in force from here on: a `/reload` (or the apply a
    // closed guided setup runs) swaps what the next read sees, and the direct
    // commands share the same handle. The session's state is shared the same
    // way, so a reload's re-seed reaches the record this launch saves.
    let live = std::sync::Arc::new(crate::front::tui::LiveRuntime::new(std::sync::Arc::clone(
        &runtime,
    )));
    // A job this surface opens runs as the old surface ran one: over the same
    // runtime, the same store, this session's folder, and the mode in force at
    // the moment the surface took the terminal — the session itself is handed
    // to the trust store below, so its policy is read here, once. The runtime is
    // the launch's live one, so a job opened after a `/reload` composes over the
    // configuration then in force, exactly as a direct command reads it.
    let jobs = std::sync::Arc::new(crate::front::tui::JobDeps::new(
        std::sync::Arc::clone(&live),
        state_db.clone(),
        session.root(),
        &state.id,
        session.policy().mode(),
    ));
    let trust = std::sync::Arc::new(crate::front::tui::direct_trust::SessionTrust::new(
        session,
        std::sync::Arc::clone(&runtime),
        launch,
    ));
    let state = std::sync::Arc::new(std::sync::Mutex::new(state));
    let direct = std::sync::Arc::new(crate::front::tui::direct_run::DirectDeps {
        runtime: std::sync::Arc::clone(&live),
        state_db: state_db.clone(),
        format,
        trust: std::sync::Arc::clone(&trust)
            as std::sync::Arc<dyn crate::front::tui::direct_trust::TrustStore>,
    });
    let outcome = run_surface(SurfaceLaunch {
        composed,
        model,
        facts,
        direct,
        jobs,
        commands,
        metrics,
        entered,
        options: cli.options.clone(),
        runtime: live,
        state: std::sync::Arc::clone(&state),
    });
    // The surface is done with the state: a reload's re-seed is already in it,
    // and what the launch saves below reads that.
    let mut state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let mut session = trust
        .release()
        .ok_or("the surface ended without giving its session back")?;
    // The surface has ended — cleanly or by cancel. The end hooks observe it,
    // each still bounded by its own timeout; a failing hook cannot change the
    // exit code.
    session.end_hooks(&runtime);
    // The terminal is restored — the surface's guard dropped when its loop
    // returned — so the last settled question's metrics section can go to
    // stderr now, never while the surface owned the screen.
    if let Some(snapshot) = session.take_question_metrics() {
        eprintln!(
            "{}",
            crate::local_metrics_format::format_snapshot(&snapshot)
        );
    }
    crate::local_metrics_format::print_metrics_section(&session.session_metrics());
    crate::chart::cleanup_session_charts();
    // The surface's own exit code: a session that ended because the operator
    // left is a clean exit, exactly as the surface this one replaces was.
    outcome?;
    let code = 0;
    if let Some(root) = session.record_root(fresh) {
        state.workspace_root = Some(root);
    }
    state.sync_host_posture(&session.universe());
    // The last save of a terminal session: a refusal at the record cap says its
    // sentence through the Error event once the terminal is restored, and keeps
    // the surface's own exit code.
    match save_with_explanation(&store, &state) {
        Err(SaveFailure::LimitReached(text)) => {
            session_emit::emit_action(SessionAction::Error(text), format, &mut state, &store)?
        }
        Err(SaveFailure::Other(error)) => return Err(error.into()),
        Ok(()) => {}
    }
    Ok(code)
}
