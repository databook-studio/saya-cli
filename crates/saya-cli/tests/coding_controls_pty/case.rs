use super::{INTERACTIVE, OUTPUT_LIMIT, PAUSE_ACK, Provider, Session, config};
use saya_store::SqliteStateStore;
use saya_types::{DatabaseProfile, JobId, JobState};

#[test]
fn selected_coding_controls_restore_terminal_and_report_truthful_receipts() {
    let mut provider = Provider::new();
    let config = config(&provider);
    let connections = "[profiles.analytics]\ntype = 'sqlite'\npath = 'analytics.sqlite3'\n\
\n[profiles.warehouse]\ntype = 'sqlite'\npath = 'warehouse.sqlite3'\n";
    let mut session = Session::new("selected", &config, connections, &provider);
    session.wait_for("? for help", INTERACTIVE);
    // The TUI must own the terminal in raw mode before input is typed: a line
    // written earlier is still under the canonical tty, whose ICRNL turns the
    // trailing \r into \n, which the raw-mode reader drops instead of
    // treating as Enter — so the command would never submit.
    // "helpers" is the request's own evidence: the lean tool set advertises
    // `helpers` only when the task asks for it, and this run drives two.
    session.send(b"/start pty-job 'controlled task with helpers' --write a.txt --write b.txt --database-profile analytics --database-profile warehouse\r");
    session.wait_for("Approve job pty-job", INTERACTIVE);
    // The overlay renders a bounded window of the proposal; the second
    // profile name is the last part this window shows, so wait for it before
    // reading the screen.
    session.wait_for("\"warehouse", INTERACTIVE);
    let approval = session.screen_text();
    assert!(
        approval.contains("\"analytics"),
        "consent proposal contains analytics identity: {approval}"
    );
    assert!(
        approval.contains("\"warehouse"),
        "consent proposal contains warehouse identity: {approval}"
    );
    // Under `ask` the panel's consent is answered by the control the prompt
    // itself names; the key modal never holds a coding approval.
    session.send(b"/approve pty-job\r");
    // One `helpers` call is one helper and waits for it, so the root holds one
    // live helper at a time: child-a is controlled first, child-b after it.
    provider.wait_for_child("child-a", INTERACTIVE);
    session.send(b"/message pty-job child-a inspect\r");
    session.wait_for("queued", INTERACTIVE);
    let pause_checkpoint = session.checkpoint();
    session.send(b"/pause pty-job child-a\r");
    session.wait_for_after(pause_checkpoint, "pause_requested", INTERACTIVE);
    let ticket = session.pause_ticket_after(pause_checkpoint, INTERACTIVE);
    session.wait_for_pause_ack(ticket, "child-a", PAUSE_ACK);
    session.send(b"/continue pty-job child-a\r");
    session.wait_for("continue_requested", INTERACTIVE);
    session.send(b"/pause pty-job missing-child\r");
    session.wait_for("not owned", INTERACTIVE);
    provider.release_child_a_answer();
    provider.wait_for_child_finished("child-a", INTERACTIVE);
    // child-b is admitted only once child-a's `helpers` call has returned.
    provider.wait_for_child("child-b", INTERACTIVE);
    session.send(b"/cancel-child pty-job child-b\r");
    session.wait_for("cancel_requested", INTERACTIVE);
    let raw = session.finish();
    let records = session.records("pty-job");
    let request = session.coding_request("pty-job");
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
    // The private records name both helpers the root handed work to; the
    // typed-command receipts (`commands-*.jsonl`) that carried
    // `"cancelled":true` are not written — a cancelled helper is proved by its
    // durable child state below.
    assert!(
        records.contains("\"child-a\"") && records.contains("\"child-b\""),
        "the private records name both helpers"
    );
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
    assert_eq!(
        request["database_profiles"],
        serde_json::json!(["analytics", "warehouse"])
    );
    let connections_scope = session
        .root()
        .join("home/saya/connections.toml")
        .canonicalize()
        .unwrap();
    let repository = session.root().join("repo").canonicalize().unwrap();
    let analytics_path = repository.join("analytics.sqlite3");
    let warehouse_path = repository.join("warehouse.sqlite3");
    let expected_identities = serde_json::json!({
        "analytics": saya_cli::profile_identity(
            "analytics",
            &DatabaseProfile::Sqlite {
                path: analytics_path.display().to_string(),
                read_only: true,
            },
            &connections_scope,
        ),
        "warehouse": saya_cli::profile_identity(
            "warehouse",
            &DatabaseProfile::Sqlite {
                path: warehouse_path.display().to_string(),
                read_only: true,
            },
            &connections_scope,
        ),
    });
    assert_eq!(request["database_identities"], expected_identities);
    provider.shutdown();
}
