/// The attached spelling on the real TUI: `/investigation run <id> --param=label=two
/// confidential words` is rejected by the parser, but history was written
/// first and the attached branch redacted only the first token, persisting
/// `--param=label=… confidential words`. The whole remainder redacts:
/// the trailing value text reaches neither the screen nor the history file
/// nor the session files, and the refusal still paints.
///
/// Counted from the real TUI, not deferred: the new surface keeps the input
/// history (`interactive::tui::history`, pushed to and recalled from by the
/// composer), echoes the submitted line — redacted — as the turn its answer
/// lands in, and settles the refusal on its own row. The attached spelling is
/// still refused by the parser, and its value text reaches neither the screen
/// nor the history file nor the session files.
#[test]
fn tui_investigation_run_attached_param_value_never_persists_or_echoes() {
    let home = scratch_home("param-attached");
    let _cleanup = HomeGuard(home.clone());

    write_param_redact_config(&home);
    // No saved investigation is needed: the parser rejects the attached
    // spelling before any id lookup, while the history write has already
    // happened — which is exactly the surface under test.
    let mut tui = match param_redact_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_investigation_run_attached_param_value: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);

    tui.send_line("/investigation run whatever --param=label=two confidential words")
        .expect("send the attached-form line");
    // The spelling is refused — pinning that this tests the rejected path.
    tui.wait_for("unknown investigation flag", INTERACTIVE)
        .expect("the attached spelling is refused by the parser");
    // Wait for the steady state before reading the screen: the draft is
    // cleared, and the redraw has blanked the cells past the shorter
    // placeholder, so the typing-time echo of the submitted line is gone.
    tui.wait_for("Ask about demo or describe a change", INTERACTIVE)
        .expect("the input draft cleared after the submit");
    tui.wait_gone("confidential words", INTERACTIVE)
        .expect("the input box cleared the submitted line");

    let screen = tui.screen();
    assert_in(
        &screen,
        "--param=label=…",
        "the transcript shows the redacted form",
        &screen,
    );
    assert_not_in(
        &screen,
        "confidential words",
        "the trailing value text must not echo on screen",
        &screen,
    );
    // The refusal itself must not carry the value either: the echoed token
    // is truncated to `--param=…` (the full message, hint included, is
    // pinned at unit level in `flags_tests.rs`).
    assert_in(
        &screen,
        "--param=…",
        "the refusal echoes the token without its value",
        &screen,
    );
    assert_not_in(
        &screen,
        "label=two",
        "the refused token's value head must not echo on screen",
        &screen,
    );

    let history = std::fs::read_to_string(home.join("input_history"))
        .expect("the TUI wrote its input history");
    assert!(
        history.contains("--param=label=…"),
        "the history keeps the redacted form: {history:?}"
    );
    assert!(
        !history.contains("confidential words"),
        "the trailing value text must not persist: {history:?}"
    );
    assert!(
        !history.contains("label=two"),
        "the refused token's value head must not persist: {history:?}"
    );

    let sessions = home.join("sessions");
    wait_for_session_file(&sessions, INTERACTIVE);
    for path in files_under(&sessions) {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !content.contains("confidential words"),
            "the trailing value text leaked into {}: {content:?}",
            path.display()
        );
        assert!(
            !content.contains("label=two"),
            "the refused token's value head leaked into {}: {content:?}",
            path.display()
        );
    }

    assert!(
        tui.still_running(),
        "the TUI is still alive after the rejected attached-form line"
    );
}

// ---------------------------------------------------------------------------
