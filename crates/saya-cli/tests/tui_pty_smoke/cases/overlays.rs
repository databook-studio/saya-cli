/// `Ctrl+K` paints `02` §11's command palette over the transcript: the pane the
/// composer's own hint row advertises (`Enter send · Ctrl+K commands`). The
/// needles are the pane's words — its title, the usage row of the selected
/// command and the key row that closes it.
#[test]
fn ctrl_k_paints_the_command_palette() {
    let home = scratch_home("surface-palette");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match surface_tui(&home, None) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping ctrl_k_paints_the_command_palette: {reason}");
            return;
        }
    };
    tui.dismiss_trust();
    // Ctrl+K is one byte on the wire: the chord the composer's hint row names.
    tui.send("\u{b}").expect("press Ctrl+K");
    tui.wait_for("Commands", INTERACTIVE)
        .expect("the palette's title paints");
    tui.wait_for("usage: /", INTERACTIVE)
        .expect("the selected command's synopsis paints");
    tui.wait_for("Esc close", INTERACTIVE)
        .expect("the palette's key row paints");
    // Esc closes it without running anything: the composer is back.
    tui.send("\x1b").expect("press Esc");
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer is back after the palette closed");
    assert!(tui.still_running(), "the surface is alive after the palette");
}

/// `?` on an empty composer paints the key sheet — the keys valid in the state
/// the operator is in, idle here: the composer's, and the two panes' own.
#[test]
fn question_mark_paints_the_help_sheet() {
    let home = scratch_home("surface-help-sheet");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match surface_tui(&home, None) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping question_mark_paints_the_help_sheet: {reason}");
            return;
        }
    };
    tui.dismiss_trust();
    tui.send("?").expect("press ?");
    tui.wait_for("Conversation", INTERACTIVE)
        .expect("the sheet's first group paints");
    // The sheet is taller than 24 rows, so its last group is a page down.
    for _ in 0..6 {
        tui.send("\x1b[6~").expect("press PageDown");
    }
    tui.wait_for("open this sheet", INTERACTIVE)
        .expect("the sheet names the key that opened it");
    tui.send("\x1b").expect("press Esc");
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer is back after the sheet closed");
    assert!(tui.still_running(), "the surface is alive after the sheet");
}
