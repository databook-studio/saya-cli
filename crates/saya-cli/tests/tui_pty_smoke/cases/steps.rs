/// The transcript's step lines on a real pty: a scripted turn runs a shell
/// command, the command's own line paints in the transcript, and `Ctrl+O` opens
/// every step in full with the outcome it reported.
///
/// Both states are reported at 100x30 and 60x24 — the two sizes `02` draws —
/// so the look is reviewable without a terminal. The screen text of the key
/// sheet is reported too, because the sheet is where the key is documented.
#[test]
fn a_turn_shows_its_command_and_ctrl_o_opens_its_outcome() {
    for (cols, rows) in [(100_u16, 30_u16), (60, 24)] {
        steps_screen(cols, rows);
    }
}

/// One size of the case: launch, run the scripted turn, and read the screen in
/// the compact layout, under `Ctrl+O`, and on the key sheet.
fn steps_screen(cols: u16, rows: u16) {
    let home = scratch_home(&format!("steps-{cols}x{rows}"));
    let _cleanup = HomeGuard(home.clone());
    let provider = StepsProvider::spawn();
    // The shell runs in the session's workspace, so the launch states one: the
    // harness's launch folder, bound before the first turn composes its tools.
    let workspace = home.join("launch-dir");
    let workspace = workspace.to_str().expect("the launch dir is a path");
    let mut tui = match surface_sized(
        &home,
        Some(&provider.base_url()),
        cols,
        rows,
        "ask",
        &["--allow-command", "echo", "--workspace", workspace],
    ) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping a_turn_shows_its_command: {reason}");
            return;
        }
    };
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer is up at launch");

    tui.send_line("say hello").expect("send the question");
    tui.wait_for(StepsProvider::COMMAND, INTERACTIVE)
        .expect("the command the step ran paints in the transcript");
    tui.wait_for(StepsProvider::ANSWER, INTERACTIVE)
        .expect("the turn settles with its answer");
    tui.wait_for("Enter send", INTERACTIVE)
        .expect("the composer is back to an idle row, so the turn has ended");
    let compact = tui.screen();
    eprintln!("--- {cols}x{rows} compact ---\n{compact}");

    assert!(
        compact.contains("Ctrl+O detail"),
        "{cols}x{rows}: the composer's hint row names the key: {compact}"
    );
    assert!(
        !compact.contains("workspace command finished"),
        "{cols}x{rows}: the compact layout keeps the outcome folded: {compact}"
    );
    assert_eq!(
        provider.requests(),
        2,
        "{cols}x{rows}: one turn is the shell call and the answer that follows it"
    );

    tui.send("\u{f}").expect("press Ctrl+O");
    tui.wait_for("workspace command finished", INTERACTIVE)
        .expect("Ctrl+O opens every step in full, the outcome under its line");
    let detail = tui.screen();
    eprintln!("--- {cols}x{rows} detail ---\n{detail}");
    assert!(
        detail.contains(StepsProvider::COMMAND),
        "{cols}x{rows}: the command stays on screen in full detail: {detail}"
    );

    tui.send("\u{f}").expect("press Ctrl+O again");
    tui.wait_for("Ctrl+O detail", INTERACTIVE)
        .expect("the composer's hint row is back to naming the key");

    tui.send("?").expect("press `?`");
    tui.wait_for("show every step in full", INTERACTIVE)
        .expect("the key sheet names the key and what it does");
    let sheet = tui.screen();
    eprintln!("--- {cols}x{rows} key sheet ---\n{sheet}");
    assert!(
        sheet.contains("Ctrl+O"),
        "{cols}x{rows}: the sheet lists the chord: {sheet}"
    );

    tui.send("\x1b").expect("press Esc");
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer is back after the sheet closed");
    assert!(
        tui.still_running(),
        "{cols}x{rows}: the surface is alive after the keys"
    );
}
