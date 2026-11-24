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
    // `/compact` touches working memory, not files or the database, so it runs
    // outside the approval engine — exactly as today's trimming does. The
    // compaction is the chat layer's one operation, reached through the
    // session's own compaction, so the summariser, the plan and the validation
    // are the ones the automatic trigger runs at a turn's boundary. A session
    // with no run open has no conversation of its own to compact: nothing ran,
    // so nothing is folded. It is stated before the loop's own parts are taken,
    // because the session it runs on is one of them.
    let said = match ctx.chat.as_ref() {
        Some(chat) => chat.compact(),
        None => crate::interactive::session::line_chat::nothing_to_compact(),
    };
    // The summary and the record this compaction saved are the session's own
    // facts: they are applied to the state here, and the cell the next request
    // is rendered from is re-read from it — the same writer, the same place,
    // as a turn's boundary compaction.
    let facts = ctx.chat.as_ref().and_then(LineChat::session_facts);
    let ActionParts {
        state,
        format,
        store,
        save_refused,
        session,
        ..
    } = action_parts(ctx);
    sync_session_facts(facts.as_deref(), session, state);
    let failed = matches!(said, Said::Error(_));
    if let Some(action) = said.action() {
        session_emit::emit_action(action, format, state, store)?;
    }
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(if failed {
        TurnOutcome::Errored
    } else {
        TurnOutcome::Completed
    })
}

fn handle_mode(
    request: crate::slash::ModeRequest,
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
    // `/mode <mode>` — the one shared switch the keyboard cycle runs too. It
    // moves the session's one policy, journals a fresh bypass activation, and
    // answers with the mode's own words; a refusal (a withheld `bypass`, a
    // frozen policy) is an errored turn and changes nothing.
    //
    // The key is read here rather than around `apply` because `apply` is pure
    // for this action: the pair the key carries moves in the switch below, so
    // the shared abandon step has to run after it — a pending clarification
    // must not answer under a mode the user changed out from under it.
    let before = state.context_key();
    let (emitted, outcome) = match crate::interactive::session::permissions::mode::switch_requested(
        state, session, request, None,
    ) {
        Ok(message) => (SessionAction::Message(message), TurnOutcome::Completed),
        Err(message) => (SessionAction::Error(message), TurnOutcome::Errored),
    };
    session_emit::emit_action(emitted, format, state, store)?;
    state.abandon_question_when_context_changed(&before, session);
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}
