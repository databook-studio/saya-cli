fn handle_preferences(
    command: crate::cli::PreferencesCommand,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        state_db,
        session,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    // `/preferences …` — the shared project-preference operations the
    // headless `saya preferences` runs, scoped to the session's bound
    // worktree root (never the launch cwd; no bound root is the shared
    // NoWorktree refusal). The words are the headless rendering, said
    // through the shared emit seam like `/grants`; a refusal is an
    // errored turn, so `--turn-file` exits non-zero.
    let (code, message) = block_on(crate::commands::preferences::run_in_session(
        command,
        session.root(),
        state_db,
    ));
    let emitted = if code == 0 {
        SessionAction::Message(message)
    } else {
        SessionAction::Error(message)
    };
    session_emit::emit_action(emitted, format, state, store)?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(if code == 0 {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    })
}

fn handle_recovery(
    command: crate::commands::recovery::RecoveryCommand,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        session,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    // `/recovery …` — the one shared `run_recovery` operation the TUI
    // runs too, over the session's own recovery handle (its store, its
    // id, its flag — the recorder's parts) and the bound workspace. A
    // refusal is an errored turn, exactly like the generic Error path,
    // so `--turn-file` exits non-zero.
    let (ok, message) = crate::commands::recovery::run_recovery(
        command,
        session.recovery(),
        session.universe().workspace().as_deref(),
        crate::interactive::session::continuation::now_unix_ms(),
    );
    let emitted = if ok {
        SessionAction::Message(message)
    } else {
        SessionAction::Error(message)
    };
    session_emit::emit_action(emitted, format, state, store)?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(if ok {
        TurnOutcome::Completed
    } else {
        TurnOutcome::Errored
    })
}

fn handle_run(
    tail: String,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        session,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // `/run --seed-grants <tail…>` seeds the child's `--allow` from this
    // session's grants on request: only tokens the run's parser accepts
    // are forwarded, the rest are named — the parent's own words, before
    // the child's stream begins. The child's parser stays the authority
    // on everything it receives. The nested run's stream passes through
    // on the real stdout/stderr (see `session_run` for the passthrough
    // rule); the child's settle message is the outcome.
    let (seed_requested, tail) = crate::interactive::session::run::separate_seed_flag(&tail);
    let seed = if seed_requested {
        let seed = crate::interactive::session::permissions::grants::run_seed(
            &session.policy().grants().tokens(),
        );
        session_emit::emit_action(
            SessionAction::Message(
                crate::interactive::session::permissions::grants::seed_message(&seed),
            ),
            format,
            state,
            store,
        )?;
        seed
    } else {
        crate::interactive::session::permissions::grants::RunSeed {
            forwarded: Vec::new(),
            dropped: Vec::new(),
        }
    };
    crate::interactive::session::run::spawn_run_child(
        runtime,
        format,
        state,
        &seed.forwarded,
        tail,
    )?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(TurnOutcome::Completed)
}

fn handle_compact(ctx: &mut TurnContext) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let ActionParts {
        state,
        runtime,
        session,
        format,
        store,
        save_refused,
        ..
    } = action_parts(ctx);
    let runtime: &RuntimeConfig = &runtime;
    // `/compact` touches working memory, not files or the database, so
    // it runs outside the approval engine — exactly as today's trimming
    // does. The summariser runs here (headless owns a `block_on` seam);
    // any failure leaves the conversation exactly as it was.
    let outcome = block_on(super::super::compact_task::run(
        runtime,
        state,
        &session.universe(),
        session.session_metrics().as_deref(),
    ));
    crate::local_metrics_format::print_metrics_section(&session.session_metrics());
    state.usage.record_learning(outcome.usage);
    // A validated manual success re-arms the automatic trigger (the TUI
    // poll clears the latch on the same validated success, and the
    // session-state doc names "any successful compaction"); a manual
    // failure or a no-work result leaves the latch exactly as it was.
    if !outcome.failed && outcome.summary.is_some() {
        state.auto_compact_failed = false;
    }
    let failed = outcome.failed;
    let action = if failed {
        SessionAction::Error(outcome.message)
    } else {
        SessionAction::Message(outcome.message)
    };
    session_emit::emit_action(action, format, state, store)?;
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(if failed {
        TurnOutcome::Errored
    } else {
        TurnOutcome::Completed
    })
}
