#[test]
fn selected_coding_controls_restore_terminal_and_report_truthful_receipts() {
    let provider = Provider::new();
    let config = config(&provider);
    let mut session = Session::new("selected", &config, &provider);
    session.send(b"/coding start pty-job 'controlled task' --write a.txt --write b.txt --program sleep --approve\r");
    session.wait_for("pty-job", INTERACTIVE);
    session.send(b"/coding message pty-job child-a inspect\r");
    session.send(b"/coding pause pty-job child-a\r");
    session.send(b"/coding continue pty-job child-a\r");
    session.send(b"/coding cancel-child pty-job child-b\r");
    session.send(b"/coding pause wrong child-a\r");
    let raw = session.finish();
    assert!(raw.len() <= OUTPUT_LIMIT);
    assert!(raw.windows(8).any(|window| window == b"\x1b[?1049l"));
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("queued"), "message was not reported queued");
    assert!(text.contains("pause_requested"));
    assert!(text.contains("pause_acknowledged") || text.contains("acknowledged"));
    assert!(text.contains("continue_requested"));
    assert!(text.contains("cancel_requested"));
    assert!(text.contains("not owned"));
    assert!(!text.contains("api_key"));
    assert!(!text.contains("password"));
    assert!(!text.contains("state.sqlite3"));
}
