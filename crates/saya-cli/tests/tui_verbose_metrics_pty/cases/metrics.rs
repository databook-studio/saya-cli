#[test]
fn tui_verbose_session_metrics_print_after_terminal_restoration() {
    warm_up_binary();
    let home = scratch_home("metrics");
    let _cleanup = HomeGuard(home.clone());
    write_offline_config(&home);
    let mut tui = spawn_raw_tui(&home).expect("the verbose TUI spawns on a pty");
    // Wait for the surface to own the terminal. A non-empty screen is not
    // enough: the CLI's own startup note (`--approval-mode`'s deprecation
    // sentence) prints on the normal screen, and a key written in that window
    // lands under the canonical tty, whose ICRNL turns Enter into Ctrl+J (the
    // jobs drawer) instead of submitting. The status line's idle word is the
    // proof the new surface is up and reading keys raw.
    tui.wait_for("? help", HARD_DEADLINE);
    // Quit through the interface's own exit command: `/` opens the command
    // palette, its query narrows to `/exit` (whose `/quit` alias the letters
    // spell), and Enter runs it — the launch answers that as the session op
    // that ends the surface (T19). Nothing has to happen on screen first.
    tui.send(b"/quit\r");
    // Drain every remaining byte, bounded. The child quit on its own, so a
    // read error past exit is the pty's end-of-stream (EIO), not a failure;
    // the grace window catches the restoration bytes after the last read.
    let start = Instant::now();
    loop {
        let exited = tui
            .child
            .child
            .try_wait()
            .is_ok_and(|status| status.is_some());
        match tui.pump() {
            Ok(true) => break,
            Ok(false) if exited && start.elapsed() >= Duration::from_millis(500) => break,
            Ok(false) => {}
            Err(_) if exited => break,
            Err(e) => panic!("{e}"),
        }
        assert!(
            start.elapsed() < HARD_DEADLINE,
            "the child never reached EOF"
        );
    }
    // EOF on the pty master arrives when the child closes its terminal, which
    // can be a few milliseconds BEFORE the process is reapable: poll for the
    // exit (bounded) instead of sampling it once at EOF.
    let exit_deadline = Instant::now() + Duration::from_secs(5);
    let exited = loop {
        if tui
            .child
            .child
            .try_wait()
            .is_ok_and(|status| status.is_some())
        {
            break true;
        }
        if Instant::now() >= exit_deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        exited,
        "the TUI child must exit on its own; the harness must never kill it"
    ); // (a) The metrics header lands only after the alternate-screen leave:
    // the terminal was restored before anything metric printed.
    let raw = tui.raw.clone();
    let enter_first = position_of(&raw, &ALT_ENTER).expect("the alternate screen was entered");
    let leave_last = rposition_of(&raw, &ALT_LEAVE).expect("the alternate screen was left");
    assert!(enter_first < leave_last, "the leave must follow the enter");
    let header_first = position_of(&raw, HEADER_TEXT)
        .expect("no metrics header anywhere in the stream: nothing printed after restoration");
    assert!(
        header_first > leave_last,
        "the metrics header must print after the terminal is restored; tail: {}",
        raw_tail(&raw)
    );
    // (b) Nothing metric-related while the TUI owned the screen: neither
    // between the enter and the leave, nor anywhere before the enter.
    let owned = &raw[enter_first..leave_last];
    assert!(
        position_of(owned, HEADER_TEXT).is_none() && position_of(owned, FIRST_PAINT_LINE).is_none(),
        "metric text printed while the TUI owned the screen; tail: {}",
        raw_tail(&raw)
    ); // (c) After the leave: the first_paint sample line, exactly once.
    let after = &raw[leave_last..];
    assert_eq!(
        count_of(after, FIRST_PAINT_LINE),
        1,
        "exactly one first_paint sample line after restoration; tail: {}",
        raw_tail(&raw)
    );
    assert_eq!(
        count_of(&raw, FIRST_PAINT_LINE),
        1,
        "no first_paint sample anywhere but the restored section; tail: {}",
        raw_tail(&raw)
    );
}
