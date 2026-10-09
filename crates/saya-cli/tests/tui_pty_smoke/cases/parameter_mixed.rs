// R061-1/R061-2 (audit 0610aaf5): mixed and attached `--param` spellings on
// the real TUI — the redaction walk must survive every submitted line and
// hide every value text from history, screen, and session files.
// ---------------------------------------------------------------------------

/// The value the R061-1 line actually binds: the parser's value zone for the
/// exact `--param` runs to the next exact run flag, so the attached-looking
/// `--param=x=y` rides inside the value — `label=hello --param=x=y` binds
/// name `label` to `hello --param=x=y`. Seeding the sentinel with exactly
/// this makes a painted row prove the original line reached execution.
const MIXED_BOUND_VALUE: &str = "hello --param=x=y";

/// R061-1 on the real TUI: `/investigation run <id> --param label=hello
/// --param=x=y` used to panic the redactor (`byte range starts at … ends at
/// …`) and kill the session with exit 101 before anything painted. Now the
/// run is a valid one — the original binding rides through — and neither
/// value text reaches the screen, the history file, or the session files.
#[test]
fn tui_investigation_run_mixed_param_tokens_survive_and_leak_nothing() {
    let home = scratch_home("param-mixed");
    let _cleanup = HomeGuard(home.clone());

    write_param_redact_config(&home);
    seed_sentinel(&home.join("demo.sqlite3"), MIXED_BOUND_VALUE);
    let id = save_param_investigation(&home);

    let mut tui = match param_redact_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_investigation_run_mixed_param_tokens: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);

    tui.send_line(&format!(
        "/investigation run {id} --param label=hello --param=x=y"
    ))
    .expect("send the mixed-token run");
    // The replay used the original binding: the value `hello --param=x=y`
    // matched the seeded sentinel row, so its id paints. A redacted line
    // reaching execution would bind `…` and paint no row.
    tui.wait_for("4242", INTERACTIVE)
        .expect("the run bound the original value (the sentinel row paints)");
    // Wait for the steady state (the draft cleared — the placeholder paints
    // exactly when the box is empty) before reading the screen, so the
    // typing-time echo of the draft never pollutes the assertion.
    tui.wait_for("Ask about your data", INTERACTIVE)
        .expect("the input draft cleared after the submit");

    let screen = tui.screen();
    assert_in(
        &screen,
        "--param label=…",
        "the transcript shows the redacted form",
        &screen,
    );
    assert_not_in(
        &screen,
        "hello",
        "the first binding's value must not echo on screen",
        &screen,
    );
    assert_not_in(
        &screen,
        "x=y",
        "the nested attached token must not echo on screen",
        &screen,
    );

    let history = std::fs::read_to_string(home.join("input_history"))
        .expect("the TUI wrote its input history");
    assert!(
        history.contains("--param label=…"),
        "the history keeps the redacted form: {history:?}"
    );
    assert!(
        !history.contains("hello"),
        "the first binding's value must not persist: {history:?}"
    );
    assert!(
        !history.contains("x=y"),
        "the nested attached token must not persist: {history:?}"
    );

    let sessions = home.join("sessions");
    wait_for_session_file(&sessions, INTERACTIVE);
    for path in files_under(&sessions) {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !content.contains("hello"),
            "the first binding's value leaked into {}: {content:?}",
            path.display()
        );
        assert!(
            !content.contains("x=y"),
            "the nested attached token leaked into {}: {content:?}",
            path.display()
        );
    }

    assert!(
        tui.still_running(),
        "the TUI is still alive after the mixed-token run"
    );
}
