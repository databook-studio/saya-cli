//! The response bound holds on the wire (A922-5, D5). `saya mcp serve`'s
//! `query` answer is narrowed by dropping whole rows until the FINAL built
//! reply fits the 16 MiB bound — rmcp 3.5.0's `CallToolResult::structured`
//! duplicates the payload into a text content block, so the wire carries
//! roughly twice the payload — with `rows`, `truncated`, and the evidence
//! row counts consistent. The real binary is driven over stdio pipes, like a
//! real MCP client; stdin is held open until the reply has arrived, because
//! requests still pending at EOF are dropped.
//!
//! Config isolation mirrors the sibling MCP tests: `--config`/`--connections`
//! point at a temp fixture and `SAYA_STATE_DB`/`SAYA_INVESTIGATIONS_DIR` at
//! temp paths, so no machine-level profile or store leaks into the
//! assertions.

use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// The published response bound (mcp/policy.rs), restated here so the test
/// fails loudly when the wire exceeds it.
const MAX_RESPONSE_BYTES: usize = 16_777_216;
/// One mebibyte of text per cell; ten cells ≈ 10 MiB of payload that the
/// payload-only check admitted whole while the wire carried ~2×.
const CELL: usize = 1_048_576;
const ROWS: u64 = 10;

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    connections: PathBuf,
    state: PathBuf,
    investigations: PathBuf,
    database: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root =
        std::env::temp_dir().join(format!("saya-cli-mcp-bound-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.sqlite3");
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let config = root.join("config.toml");
    std::fs::write(&config, "default_profile = 'local'\n").unwrap();
    let state = root.join("state.sqlite3");
    let investigations = root.join("investigations");
    std::fs::create_dir_all(&investigations).unwrap();
    std::fs::write(&database, b"").unwrap();
    Fixture {
        root,
        config,
        connections,
        state,
        investigations,
        database,
    }
}

impl Fixture {
    /// Seeds the `blobs` table through sqlx, directly (the connector is
    /// read-only; only the test writes).
    fn seed_blobs(&self) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&self.database)
                    .create_if_missing(true);
                let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
                sqlx::query("CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload TEXT NOT NULL)")
                    .execute(&pool)
                    .await
                    .unwrap();
                for id in 1..=ROWS {
                    sqlx::query("INSERT INTO blobs (id, payload) VALUES (?, ?)")
                        .bind(id as i64)
                        .bind("x".repeat(CELL))
                        .execute(&pool)
                        .await
                        .unwrap();
                }
                pool.close().await;
            });
    }
}

struct TestServer {
    stdin: Option<ChildStdin>,
    responses: Receiver<String>,
    child: Child,
}

impl TestServer {
    fn spawn(fixture: &Fixture, extra: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_saya"))
            .args([
                "--config",
                fixture.config.to_str().unwrap(),
                "--connections",
                fixture.connections.to_str().unwrap(),
            ])
            .args(extra)
            .env("SAYA_STATE_DB", &fixture.state)
            .env("SAYA_INVESTIGATIONS_DIR", &fixture.investigations)
            // Hermetic config discovery: the user layer is this fixture's
            // home only, never the running machine's.
            .env("HOME", fixture.root.join("home"))
            .env("XDG_CONFIG_HOME", fixture.root.join("home").join(".config"))
            .env_remove("SAYA_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the saya binary");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            stdin: Some(stdin),
            responses,
            child,
        }
    }

    fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin still open");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn send_json(&mut self, frame: Value) {
        self.send(&serde_json::to_string(&frame).unwrap());
    }

    /// The next stdout line raw (the wire bound is asserted on the line
    /// bytes before any parsing); panics past the deadline so a hung server
    /// fails the test instead of stalling the suite.
    fn next_line(&self, seconds: u64) -> String {
        match self.responses.recv_timeout(Duration::from_secs(seconds)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => panic!("no server response within {seconds}s"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("server stdout closed before the expected response");
            }
        }
    }

    fn handshake(&mut self) {
        self.send_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "bound-test", "version": "0"},
            },
        }));
        let init: Value =
            serde_json::from_str(&self.next_line(10)).expect("initialize answers with JSON");
        assert_eq!(
            init["result"]["protocolVersion"], PROTOCOL_VERSION,
            "the requested protocol revision is echoed"
        );
        self.send_json(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }

    /// One `query` round-trip; stdin stays open across it (a request pending
    /// at EOF is dropped). Returns the raw reply LINE.
    fn call_query(&mut self, id: i64, sql: &str) -> String {
        self.send_json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "query",
                "arguments": {"profile": "local", "sql": sql},
            },
        }));
        let line = self.next_line(120);
        let reply: Value = serde_json::from_str(&line).expect("the reply line is JSON");
        assert_eq!(reply["id"], id, "the reply answers this call");
        line
    }

    /// Close stdin and require exit 0 within 15 s (a parallel test run can
    /// starve this thread's wall clock, and the last frame was megabytes).
    fn close_and_expect_exit(&mut self, expected: i32) {
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                assert_eq!(
                    status.code(),
                    Some(expected),
                    "server exit code after stdin EOF"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "server did not exit within 5s of stdin EOF"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// A query whose payload alone fits the 16 MiB bound but whose built reply
/// (the duplicated text content block) does not: the reply LINE on the wire
/// stays within the bound, narrowed by dropping whole rows — `truncated`,
/// the row counts, and the evidence all agree — and the server exits cleanly.
#[test]
fn mcp_query_reply_line_stays_within_the_wire_bound() {
    let fixture = fixture("wire");
    fixture.seed_blobs();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();

    let reply_line = server.call_query(2, "SELECT id, payload FROM blobs ORDER BY id");

    assert!(
        reply_line.len() <= MAX_RESPONSE_BYTES,
        "the reply LINE must stay within the {}-byte wire bound, was {} bytes",
        MAX_RESPONSE_BYTES,
        reply_line.len()
    );
    let reply: Value = serde_json::from_str(&reply_line).expect("the reply line is JSON");
    assert_eq!(
        reply["result"]["isError"], false,
        "the oversized answer is narrowed and sent, not refused"
    );
    let payload = &reply["result"]["structuredContent"];
    assert_eq!(
        payload["truncated"], true,
        "the answer is marked truncated: {}",
        payload["note"]
    );
    let sent = payload["row_count"]
        .as_u64()
        .expect("row_count is a number");
    assert!(
        sent < ROWS,
        "rows were dropped to fit the wire: {sent} of {ROWS}"
    );
    assert_eq!(
        payload["rows"].as_array().expect("rows array").len() as u64,
        sent,
        "row_count matches the rows actually sent"
    );
    assert_eq!(
        payload["evidence"]["returned_rows"], sent,
        "the evidence reports the rows actually sent"
    );
    assert_eq!(
        payload["evidence"]["truncated"], true,
        "the evidence reports the narrowing"
    );
    assert!(
        payload["note"]
            .as_str()
            .unwrap_or_default()
            .contains("cut to"),
        "the narrowing is named: {}",
        payload["note"]
    );

    server.close_and_expect_exit(0);
}
