//! MVP test (B3c): the scripted provider stops to ask one focused question
//! instead of answering, and the `saya ask` process must exit with the
//! paused-for-input class — 6 — with the question on the stream.

use crate::common::drain_request;
use std::{fs, io::Write, net::TcpListener, process::Command as ProcessCommand, thread};

#[test]
fn ask_exits_six_when_the_model_stops_to_ask() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let args = serde_json::json!({
        "question": "Which time column defines active?",
        "options": ["signup_date", "last_order_date"]
    });
    let call = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [
            {"index": 0, "id": "call_ask",
             "function": {"name": "request_clarification", "arguments": args.to_string()}}
        ]}}]
    });
    let body = format!("data: {call}\n\ndata: [DONE]\n\n");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        drain_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        )
        .unwrap();
        stream.flush().unwrap();
    });
    let root = std::env::temp_dir().join(format!("saya-cli-ask-clarify-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let state_db = root.join("state.sqlite3");
    let database = root.join("ask.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE revenue (amount INTEGER);")
        .unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "ndjson",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "ask",
            "show revenue",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &state_db)
        .output()
        .unwrap();
    handle.join().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(6),
        "the paused class is exit 6. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(r#""event":"clarification_needed""#),
        "the ndjson stream carries the structured event: {stdout}"
    );
    assert!(
        stdout.contains("Which time column defines active?"),
        "the question is on the stream: {stdout}"
    );
    assert!(
        stdout.contains("signup_date") && stdout.contains("last_order_date"),
        "the options ride the stream: {stdout}"
    );
    assert!(output.stderr.is_empty(), "{stdout}");
    let _ = fs::remove_dir_all(root);
}
