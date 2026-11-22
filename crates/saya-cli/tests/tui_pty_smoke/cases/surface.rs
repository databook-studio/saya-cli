/// The launch switch's first proof: the real binary, on a pty, opens the NEW
/// surface — its status line, its composer and the startup trust question
/// inside the pane — and never the old one.
///
/// The needles are the new surface's own words: `? help` is the status line's
/// idle word (`view::status::ActivityState`), `read-only` is the mode chip the
/// launch's `--approval-mode` states, `Message` is the composer's title and
/// `Enter send` its hint row (`view::composer`), and `trust this folder?` is
/// the trust pane's title (`view::trust::TITLE`, unchanged from the old modal
/// because the words are the one trust module's).
#[test]
fn the_binary_on_a_pty_opens_the_new_surface() {
    let home = scratch_home("surface-open");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match surface_tui(&home, None) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping the_binary_on_a_pty_opens_the_new_surface: {reason}");
            return;
        }
    };
    tui.wait_for("? help", INTERACTIVE)
        .expect("the new status line paints its idle word");
    tui.wait_for("read-only", INTERACTIVE)
        .expect("the status line's mode chip names the launch's mode");
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer's box paints");
    tui.wait_for("Enter send", INTERACTIVE)
        .expect("the composer's hint row paints");
    tui.wait_for("trust this folder?", INTERACTIVE)
        .expect("the trust question rides the interface, never a raw stdin read");
    tui.dismiss_trust();
    tui.wait_for("no workspace", INTERACTIVE)
        .expect("the status line names the unbound workspace");
    assert!(tui.still_running(), "the surface is alive after its first frame");
    // The old surface's splash words must be nowhere: this process opened the
    // new one, and neither surface is reachable from the other.
    let screen = tui.screen();
    assert!(
        !screen.contains("? for help"),
        "the old status bar painted: {screen}"
    );
    assert!(
        !screen.contains("AI        ready"),
        "the old splash painted: {screen}"
    );
}

/// `/help` paints the command listing: the chat layer's one table answers the
/// read (`chat::ops`), and the surface lands the words in the transcript.
#[test]
fn help_paints_the_command_listing() {
    let home = scratch_home("surface-help");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match surface_tui(&home, None) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping help_paints_the_command_listing: {reason}");
            return;
        }
    };
    tui.dismiss_trust();
    tui.send_line("/help").expect("send /help");
    // The listing is longer than the pane, and the transcript keeps its newest
    // rows: the tail of the Session category is what the operator sees, and its
    // `help` row is the proof the listing came from the catalogue.
    tui.wait_for("Show help for slash commands", INTERACTIVE)
        .expect("a catalogue row paints");
    tui.wait_for("Exit the REPL", INTERACTIVE)
        .expect("the listing's exit row paints");
    assert!(tui.still_running(), "the surface is alive after /help");
}

/// `/exit` leaves the surface and restores the terminal: the child ends on its
/// own, and the pty it drew on is back in canonical mode — no raw mode left
/// behind.
#[test]
fn exit_restores_the_terminal_and_leaves_no_raw_mode() {
    let home = scratch_home("surface-exit");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match surface_tui(&home, None) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping exit_restores_the_terminal: {reason}");
            return;
        }
    };
    tui.dismiss_trust();
    tui.send_line("/exit").expect("send /exit");
    tui.wait_exit(INTERACTIVE)
        .expect("the surface left on its own after /exit");
    assert!(
        tui.canonical(),
        "the terminal is back in canonical mode: no raw mode left"
    );
}
