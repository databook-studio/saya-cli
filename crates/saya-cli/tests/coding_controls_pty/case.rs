use super::{INTERACTIVE, OUTPUT_LIMIT, Provider, Session, config};
use saya_store::SqliteStateStore;
use saya_types::{JobId, JobState};

#[test]
fn selected_coding_controls_restore_terminal_and_report_truthful_receipts() {
    let mut provider = Provider::new();
    let config = config(&provider);
    let mut session = Session::new("selected", &config, &provider);
    session.wait_for("? for help", INTERACTIVE);
    // The TUI must own the terminal in raw mode before input is typed: a line
    // written earlier is still under the canonical tty, whose ICRNL turns the
    // trailing \r into \n, which the raw-mode reader drops instead of
    // treating as Enter — so the command would never submit.
    session.send(b"/coding start pty-job 'controlled task' --write a.txt --write b.txt --program sleep --approve\r");
    provider.wait_for_children(2, INTERACTIVE);
    session.send(b"/coding message pty-job child-a inspect\r");
    session.wait_for("queued", INTERACTIVE);
    let pause_checkpoint = session.checkpoint();
    session.send(b"/coding pause pty-job child-a\r");
    session.wait_for_after(pause_checkpoint, "pause_requested", INTERACTIVE);
    let ticket = session.pause_ticket_after(pause_checkpoint, INTERACTIVE);
    session.wait_for_pause_ack(pause_checkpoint, ticket, "child-a", INTERACTIVE);
    session.send(b"/coding continue pty-job child-a\r");
    session.wait_for("continue_requested", INTERACTIVE);
    session.send(b"/coding cancel-child pty-job child-b\r");
    session.wait_for("cancel_requested", INTERACTIVE);
    session.send(b"/coding pause pty-job missing-child\r");
    session.wait_for("not owned", INTERACTIVE);
    provider.wait_for_child_finished("child-a", INTERACTIVE);
    let raw = session.finish();
    let records = session.records("pty-job");
    let children = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            SqliteStateStore::new(session.state_db())
                .list_coding_children(&JobId::parse("pty-job").unwrap(), None, 100)
                .await
                .unwrap()
                .items
        });
    assert!(raw.len() <= OUTPUT_LIMIT);
    let enter = raw.windows(8).rposition(|window| window == b"\x1b[?1049h");
    let leave = raw.windows(8).rposition(|window| window == b"\x1b[?1049l");
    let marker = raw
        .windows(b"saya local metrics".len())
        .position(|window| window == b"saya local metrics");
    assert!(enter.is_some(), "alternate screen was entered");
    assert!(leave > enter, "alternate screen leaves after entering");
    assert!(
        marker > leave,
        "post-run marker follows terminal restoration"
    );
    assert!(
        session.child_reaped(),
        "PTY child was reaped before finish returned"
    );
    assert!(
        session.reader_finished(),
        "PTY reader reached EOF/is_finished before finish returned"
    );
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("queued"), "message was not reported queued");
    assert!(text.contains("pause_requested"));
    assert!(text.contains("pause_acknowledged"));
    assert!(text.contains("pause_acknowledged\":true"));
    assert!(text.contains("continue_requested"));
    assert!(text.contains("cancel_requested"));
    assert!(text.contains("not owned"));
    assert!(records.contains("\"joined\":true"));
    assert!(records.contains("\"cancelled\":true"));
    let child_a = children
        .iter()
        .find(|child| child.agent_id.as_str() == "child-a")
        .expect("durable child-a completion exists");
    let child_b = children
        .iter()
        .find(|child| child.agent_id.as_str() == "child-b")
        .expect("durable child-b cancellation exists");
    assert_eq!(child_a.state, JobState::Completed);
    assert_eq!(child_b.state, JobState::Cancelled);
    assert!(!text.contains("api_key"));
    assert!(!text.contains("password"));
    assert!(!text.contains("state.sqlite3"));
    provider.shutdown();
}
