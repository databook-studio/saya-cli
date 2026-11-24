// The launch surface `--plain` states. On a terminal the launch opens the new
// full-screen TUI (T19); `--plain` — or the same request through the
// environment — opens the line REPL the piped path already runs, over the same
// chat session. The three cases are one decision: the flag chooses, the
// environment states the same thing, and stating neither leaves the TUI where
// T19 put it.

/// The one walk to a plain launch's prompt. The startup trust question is the
/// LINE prompt on this surface — asked before the loop, on stderr, never as the
/// TUI's pane — so it is answered `c` (continue unbound) here, and the loop's
/// own status header and `saya> ` marker follow. Every positive assertion lives
/// in this one place, so the two plain cases cannot disagree about what the
/// line REPL prints. The prompt needle is `saya>` without its trailing space:
/// `screen_text` trims each row's right edge, so the marker's own space is
/// never part of the settled text.
fn settle_line_repl(tui: &mut InteractiveTui, label: &str) -> String {
    tui.wait_for("Trust this folder as the session workspace?", INTERACTIVE)
        .unwrap_or_else(|error| {
            panic!("{label}: the plain surface asks the line trust prompt: {error}")
        });
    tui.send_line("c").expect("the trust answer");
    tui.wait_for("saya>", INTERACTIVE)
        .unwrap_or_else(|error| panic!("{label}: the line REPL prompt: {error}"));

    let text = tui.screen();
    assert_in(
        &text,
        "[demo]",
        &format!("{label}: the line status header names the profile"),
        &text,
    );
    assert_in(
        &text,
        "approval:read-only",
        &format!("{label}: the line status header names the approval mode"),
        &text,
    );
    // Negative: the full-screen TUI's own furniture must not be on this screen.
    // The keyboard hint `? help` and the `│`-separated status row are the new
    // surface's words (T19); seeing either means the launch still chose it.
    assert_not_in(
        &text,
        "? help",
        &format!("{label}: the TUI's keyboard hint painted under a plain launch"),
        &text,
    );
    assert_not_in(
        &text,
        "read-only │ smoke │ demo",
        &format!("{label}: the full-screen status row painted under a plain launch"),
        &text,
    );
    text
}

#[test]
fn plain_flag_opens_the_line_repl_on_a_pty() {
    let home = scratch_home("plain-flag");
    let _cleanup = HomeGuard(home.clone());

    let mut tui = match interactive_launch(&home, &["--plain"], &[]) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_pty_smoke (--plain): {reason}");
            return;
        }
    };
    settle_line_repl(&mut tui, "--plain");
    assert!(
        tui.still_running(),
        "the line REPL is a running loop, not a launch that exited"
    );
}

#[test]
fn saya_plain_env_opens_the_line_repl_on_a_pty() {
    let home = scratch_home("plain-env");
    let _cleanup = HomeGuard(home.clone());

    let mut tui = match interactive_launch(&home, &[], &[("SAYA_PLAIN", "1")]) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_pty_smoke (SAYA_PLAIN): {reason}");
            return;
        }
    };
    settle_line_repl(&mut tui, "SAYA_PLAIN=1");
}

#[test]
fn no_plain_request_still_opens_the_full_screen_tui_on_a_pty() {
    let home = scratch_home("plain-absent");
    let _cleanup = HomeGuard(home.clone());

    let mut tui = match interactive_launch(&home, &[], &[]) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_pty_smoke (no plain request): {reason}");
            return;
        }
    };
    tui.wait_for("? help", INTERACTIVE)
        .expect("an unstated launch opens the full-screen TUI");
    let text = tui.screen();
    assert_in(
        &text,
        "read-only │ smoke │ demo",
        "the full-screen status row paints when no plain request is stated",
        &text,
    );
    assert_not_in(
        &text,
        "saya>",
        "the line loop's prompt must not paint in the full-screen TUI",
        &text,
    );
}
