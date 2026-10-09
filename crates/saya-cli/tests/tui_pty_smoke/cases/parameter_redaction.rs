/// `/investigation run` with `--param` on the real TUI (D2 / A922-2): the
/// value drives the run — the param-bound row paints — but never persists or
/// echoes: not on the screen, not in the `input_history` file, not in the
/// session files under `SAYA_SESSION_DIR`. The persisted copy (and what
/// Up-arrow recalls) shows `--param label=…` — the user retypes values.
#[test]
fn tui_investigation_run_param_value_never_persists_or_echoes() {
    let home = scratch_home("param-redact");
    let _cleanup = HomeGuard(home.clone());

    write_param_redact_config(&home);
    seed_sentinel(&home.join("demo.sqlite3"), PARAM_VALUE);
    let id = save_param_investigation(&home);

    let mut tui = match param_redact_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_investigation_run_param_redact: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);

    tui.send_line(&format!(
        "/investigation run {id} --param label={PARAM_VALUE}"
    ))
    .expect("send the parameterized run");
    // The run used the value: the bound query's row paints — a refused run
    // (the value never reached the binding) would show no such table.
    tui.wait_for("match_id", INTERACTIVE)
        .expect("the param-bound run's table paints");
    tui.wait_for("4242", INTERACTIVE)
        .expect("the param-bound row paints");

    // The screen never echoes the value.
    let screen = tui.screen();
    assert_not_in(
        &screen,
        PARAM_VALUE,
        "the submitted value must not echo on screen",
        &screen,
    );

    // Up-arrow recall: the persisted entry is the redacted form (the
    // accepted trade), never the value.
    tui.send("\x1b[A").expect("press Up to recall");
    tui.wait_for(PARAM_REDACTED, INTERACTIVE)
        .expect("recall shows the redacted form");
    let recalled = tui.screen();
    assert_not_in(
        &recalled,
        PARAM_VALUE,
        "recall must not resurrect the value",
        &recalled,
    );

    // The input_history file: the redacted line, never the value.
    let history = std::fs::read_to_string(home.join("input_history"))
        .expect("the TUI wrote its input history");
    assert!(
        history.contains(PARAM_REDACTED),
        "the history keeps the redacted form: {history:?}"
    );
    assert!(
        !history.contains(PARAM_VALUE),
        "the value must not persist in input_history: {history:?}"
    );

    // The session files under SAYA_SESSION_DIR: the value is nowhere on disk.
    let sessions = home.join("sessions");
    wait_for_session_file(&sessions, INTERACTIVE);
    for path in files_under(&sessions) {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !content.contains(PARAM_VALUE),
            "the value leaked into {}: {content:?}",
            path.display()
        );
    }

    assert!(
        tui.still_running(),
        "the TUI is still alive after the parameterized run"
    );
}

// ---------------------------------------------------------------------------
