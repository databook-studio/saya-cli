/// `/setup` answered with EOF at the first prompt: the wizard cancels
/// ("No files were changed.", exit 0), the TUI takes the terminal back and
/// says so, input works again, and the raw stream shows the exact
/// enter → leave → wizard → enter ordering.
#[test]
fn setup_handoff_cancel_returns_to_a_responsive_tui() {
    let home = scratch_home("setup-cancel");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match setup_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping setup_handoff_cancel: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);
    // Record the CPR count while the TUI is idle: the handoff below must add
    // none (production no longer depends on a cursor-position reply).
    let cpr_before_handoff = count_of(&tui.raw, CURSOR_QUERY);

    tui.send_line("/setup").expect("send /setup");
    // The first Enter is consumed by the completion popup (a bare command's
    // accept), so a second one submits.
    tui.send("\r").expect("submit /setup");
    tui.wait_for(FIRST_PROMPT, INTERACTIVE)
        .expect("the wizard took the terminal");
    tui.send("\x04").expect("send EOF");
    tui.wait_for("Guided setup closed", INTERACTIVE)
        .expect("the TUI took the terminal back after the cancel");
    // U07: the automatic reload reports the unchanged configuration.
    tui.wait_for("Configuration is unchanged.", INTERACTIVE)
        .expect("the reload reports unchanged after the cancel");
    // The handoff re-entered the alternate screen without querying the
    // cursor position: the CPR count is unchanged.
    assert_eq!(
        count_of(&tui.raw, CURSOR_QUERY),
        cpr_before_handoff,
        "the handoff added no cursor-position query"
    );
    // The wizard's cancel line printed in the normal screen: it must be in
    // the raw stream even though the TUI's repaint switched the parser back
    // to the alternate screen.
    assert!(
        position_of(&tui.raw, b"No files were changed.").is_some(),
        "the wizard's cancel line was printed"
    );
    tui.send_line("/help").expect("send /help");
    tui.send("\r").expect("submit /help");
    tui.wait_for("List runs, or show one run by id", INTERACTIVE)
        .expect("input works again after the handoff");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the cancel"
    );
    assert_raw_handoff_ordering(&tui);
}
