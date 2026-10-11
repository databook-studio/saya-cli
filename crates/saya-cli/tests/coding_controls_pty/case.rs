use super::{INTERACTIVE, OUTPUT_LIMIT, Provider, Session, config};
use saya_store::SqliteStateStore;
use saya_types::{JobId, JobState};

#[test]
fn selected_coding_controls_restore_terminal_and_report_truthful_receipts() {
    let provider = Provider::new();
    let config = config(&provider);
    let mut session = Session::new("selected", &config, &provider);
    session.send(b"/coding start pty-job 'controlled task' --write a.txt --write b.txt --program sleep --approve\r");
    provider.wait_for_children(2, INTERACTIVE);
    session.send(b"/coding message pty-job child-a inspect\r");
    session.wait_for("queued", INTERACTIVE);
    session.send(b"/coding pause pty-job child-a\r");
    session.wait_for("pause_requested", INTERACTIVE);
    session.wait_for("pause_acknowledged", INTERACTIVE);
    session.send(b"/coding continue pty-job child-a\r");
    session.wait_for("continue_requested", INTERACTIVE);
    session.send(b"/coding cancel-child pty-job child-b\r");
    session.wait_for("cancel_requested", INTERACTIVE);
    session.send(b"/coding pause pty-job missing-child\r");
    session.wait_for("not owned", INTERACTIVE);
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
    let enter = raw.windows(8).position(|window| window == b"\x1b[?1049h");
    let leave = raw.windows(8).position(|window| window == b"\x1b[?1049l");
    let marker = raw
        .windows(b"saya local metrics".len())
        .position(|window| window == b"saya local metrics");
    assert!(enter.is_some(), "alternate screen was entered");
    assert!(leave > enter, "alternate screen leaves after entering");
    assert!(
        marker > leave,
        "post-run marker follows terminal restoration"
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
}
