/// Runs exactly one turn through the session's one per-turn entry
/// (`handle_line`), preserving the file bytes verbatim: no line splitting,
/// no trimming, no blank-line skipping. Returns the turn outcome so the
/// caller maps it to the process exit code.
fn run_single_turn(
    turn: String,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    // The per-turn entry reports its outcome directly. Do not infer success
    // from a change in persisted vector lengths: valid commands such as
    // `/schema` can complete without appending a session message, while an
    // error can leave unrelated state untouched.
    handle_line_verbatim(&turn, ctx)
}

/// The one terminal result a scripted run reports: under json/ndjson the
/// machine-readable `turn_result` event ends stdout; in text mode only the
/// unfinished-tools warning is said, on stderr, and only when a cancelled
/// turn may have left a write's effects unknown. The `exit_code` is the
/// value the process returns: the caller computes it once from the final
/// outcome and passes the same value here. A stdout write error surfaces
/// exactly as `print!` surfaces it today; the emission never fails the run.
fn emit_turn_result(format: RenderFormat, status: TurnStatus, exit_code: i32, tally: ToolTally) {
    let unfinished = tally.unfinished_entries();
    let rendered = crate::render::render_turn::turn_result_rendered(
        format,
        status,
        exit_code,
        tally.completed(),
        &unfinished,
    );
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
}

/// Reads lines from stdin without the rich editor, printing the status header
/// and `saya> ` marker when attached to a terminal. Used for piped input and as
/// a graceful fallback when the rich editor cannot initialize.
fn run_plain_loop(ctx: &mut TurnContext) -> Result<(), Box<dyn std::error::Error>> {
    // A startup fact the user must read once, in the loop they will see every
    // turn: a pinned root that vanished, or any other composition notice —
    // and, under bypass, the mode's activation line with its no-euphemism
    // wording and the probe/absence facts. Said as a diagnostic: the loop's
    // stdout contract is the turn stream (JSON under `--format`), and a bare
    // notice line would corrupt it.
    if let Some(notice) = ctx.session.notice() {
        eprintln!("{notice}");
    }
    if let Some(echo) = ctx.trusted_echo.as_deref() {
        eprintln!("{echo}");
    }
    if let Some(line) = crate::interactive::session::permissions::activation::line_if_bypass(
        ctx.state,
        &ctx.runtime,
        &ctx.session.universe(),
    ) {
        let rendered =
            crate::render_event(&TerminalEvent::Diagnostic { message: line }, ctx.format);
        print!("{}", rendered.stdout);
        eprint!("{}", rendered.stderr);
    }
    // Bypass × unbound × non-terminal: no prompt was possible, so nothing
    // bound and the lane cannot compose either — bypass runs with the lane
    // absent, and the notice says so (the fail-closed intersection). Said
    // as a diagnostic beside the other startup facts: the turn stream owns
    // stdout, and under `--format json` a bare line would corrupt it.
    if let Some(note) = crate::interactive::session::permissions::trust::bypass_no_lane_note(
        crate::interactive::session::permissions::activation::is_bypass_mode(ctx.state),
        ctx.session.universe().host_composed(),
    ) {
        eprintln!("{note}");
    }
    let mut input = String::new();
    loop {
        if ctx.terminal {
            println!("{}", super::super::session_prompt::status_line(ctx.state));
            print!("saya> ");
            io::stdout().flush()?;
        }
        input.clear();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }
        if handle_line(&input, ctx)? {
            break;
        }
    }
    Ok(())
}

/// Processes one input line: dispatches a slash command or an agent prompt,
/// renders the resulting action, and persists the redacted session. Returns
/// `Ok(true)` when the session should exit.
fn handle_line(line: &str, ctx: &mut TurnContext) -> Result<bool, Box<dyn std::error::Error>> {
    // Piped stdin reads line by line: trim the line ending here, keep the
    // blank-line skip in the shared entry below so both paths share it.
    Ok(matches!(
        handle_line_verbatim(line.trim_end(), ctx)?,
        TurnOutcome::Exit
    ))
}

/// The one save every headless turn ends with. A save refused at the 4 MiB
/// record cap says its friendly sentence as the turn's Error event, records
/// the refusal through `save_refused` (so the scripted run's end-of-run
/// save never repeats the identical sentence), and the turn ends errored
/// while the loop keeps reading — it must not die; any other store error
/// stays fatal, exactly as before. After a successful save the one-time
/// near-limit notice, if it fires, rides a result event — ahead of the
/// scripted run's closing `turn_result`. `Ok(Some(outcome))` overrides the
/// call site's own outcome; `Ok(None)` — the save succeeded — leaves it.
fn persist(
    state: &mut SessionState,
    store: &FsSessionStore,
    format: RenderFormat,
    save_refused: &mut bool,
) -> Result<Option<TurnOutcome>, Box<dyn std::error::Error>> {
    match save_with_explanation(store, state) {
        Err(SaveFailure::LimitReached(text)) => {
            *save_refused = true;
            session_emit::emit_action(SessionAction::Error(text), format, state, store)?;
            return Ok(Some(TurnOutcome::Errored));
        }
        Err(SaveFailure::Other(error)) => return Err(error.into()),
        Ok(()) => {}
    }
    if let Some(text) = notice_after_save(store, state) {
        session_emit::emit_action(SessionAction::Message(text), format, state, store)?;
    }
    Ok(None)
}
