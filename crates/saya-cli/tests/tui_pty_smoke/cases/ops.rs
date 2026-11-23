// The catalogue commands on the real surface: a command chosen from the
// palette is *performed* here, not handed to another surface. The finding was
// that the palette offered the whole catalogue while the full-screen surface
// answered "that command is the line front's in this release"; this case drives
// one of those commands from a real terminal and reads the panel it must paint.

/// `Ctrl+K`, `/status`, and the panel's sections on the screen. The palette is
/// the surface's own (the same one `/` opens), the command is one of the ones
/// that used to defer, and the evidence is the panel the session itself builds.
#[test]
fn tui_palette_status_paints_the_panel() {
    let home = scratch_home("palette-status");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match interactive_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_palette_status: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);

    // `Ctrl+K` is the palette's own chord (`02` §11), so the command is chosen
    // the way an operator chooses it without a slash.
    tui.send("\x0b").expect("open the command palette");
    tui.send("status\r").expect("choose /status");
    tui.send("\r").expect("submit /status");

    // The panel the session builds, scrolled to its newest line the way a
    // transcript always is: its last section is on screen, and the deferral it
    // used to answer with is nowhere near.
    tui.wait_for("Limits", INTERACTIVE)
        .expect("the panel's last section paints");
    assert!(
        !tui.screen().contains("the line front's"),
        "/status is performed here, never deferred: {}",
        tui.screen()
    );
    assert!(
        tui.still_running(),
        "the TUI is still alive after the read"
    );
}
