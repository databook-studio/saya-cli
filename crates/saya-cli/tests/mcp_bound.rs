//! The response bound holds on the wire (A922-5, D5; D9). `saya mcp serve`'s
//! `query` answer is narrowed by dropping whole rows until the FINAL reply
//! LINE — the JSON-RPC envelope with the client's ACTUAL request id wrapped
//! around the built result, plus the writer's newline — fits the 16 MiB
//! bound; rmcp 3.5.0's `CallToolResult::structured` duplicates the payload
//! into a text content block, so the wire carries roughly twice the payload,
//! and a large client-controlled id rides the envelope on top. `rows`,
//! `truncated`, and the evidence row counts stay consistent. The real binary
//! is driven over stdio pipes, like a real MCP client; stdin is held open
//! until the reply has arrived, because requests still pending at EOF are
//! dropped.
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
        self.seed_blobs_count(ROWS);
    }

    /// The same, with an explicit row count: the engine-truncation case needs
    /// more rows than the connector's own 16 MiB result cap will return.
    fn seed_blobs_count(&self, rows: u64) {
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
                for id in 1..=rows {
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
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                // read_until keeps the terminating newline: the wire bound is
                // asserted on the reply line's exact bytes, newline included.
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf).into_owned();
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
    /// at EOF is dropped). The id may be any JSON-RPC id shape — number or
    /// string; the server echoes it in the reply envelope. Returns the raw
    /// reply LINE with its trailing newline.
    fn call_query(&mut self, id: Value, sql: &str) -> String {
        self.call_query_with_meta(id, None, sql)
    }

    /// The same, with `meta` riding the call params as `_meta` (None keeps
    /// the legacy wire shape): rmcp only keeps the `resultType` discriminator
    /// on the reply when the request itself declares a 2026-07-28+ protocol.
    fn call_query_with_meta(&mut self, id: Value, meta: Option<Value>, sql: &str) -> String {
        let mut params = json!({
            "name": "query",
            "arguments": {"profile": "local", "sql": sql},
        });
        if let Some(meta) = meta {
            params["_meta"] = meta;
        }
        self.send_json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": params,
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

    let reply_line = server.call_query(json!(2), "SELECT id, payload FROM blobs ORDER BY id");

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

/// The audit's reproduction SQL (R061-3): eight rows of a 1,040,000-char
/// cell — each cell rides just under the connector's 1 MiB cap, and the
/// duplicated text content block carries the built result to ~16.64 MB:
/// inside the old fixed-reserve measure (which admitted it whole) but over
/// the true reply line for any large request id.
const AUDIT_SQL: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n \
                         WHERE x<8) SELECT printf('%1040000s','x') AS cell FROM n";

/// The audit's reproduction `_meta` (R29-1): a `tools/call` that declares the
/// 2026-07-28 protocol inline, so rmcp 3.5.0 keeps the `resultType`
/// discriminator on the reply and the measured budget matches the wire with
/// no legacy slack. The client-capabilities key is required by the same
/// protocol draft for a request that declares its version inline.
fn audit_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

/// Every admitted request id must get a reply line within the bound (D9).
/// The three id shapes below exercise the envelope's real serialization:
/// plain ASCII (the audit's case), a quote/backslash id whose JSON escaping
/// doubles its wire size, and a non-ASCII id carried as UTF-8 bytes.
/// Assertions: the reply LINE (newline included) fits the bound, the answer
/// is narrowed — never refused — and `rows`/`row_count`/`truncated`/the
/// evidence counts all agree.
fn assert_reply_line_fits_and_stays_consistent(reply_line: &str) {
    assert!(
        reply_line.len() <= MAX_RESPONSE_BYTES,
        "the reply LINE must stay within the {}-byte wire bound, was {} bytes",
        MAX_RESPONSE_BYTES,
        reply_line.len()
    );
    let reply: Value = serde_json::from_str(reply_line).expect("the reply line is JSON");
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
        sent < 8,
        "rows were dropped to fit the wire: {sent} of the 8 the SQL returns"
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
}

/// The audit's exact case (R061-3, D9): a 200,000-character ASCII request id
/// rides the reply envelope, so the built result the old fixed-reserve
/// measure admitted whole produced a 16,841,216-byte reply line — 64,000
/// bytes over the bound. The budget must come from the actual envelope.
#[test]
fn mcp_query_reply_line_holds_with_the_audit_large_ascii_request_id() {
    let fixture = fixture("id-ascii");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line = server.call_query(json!("i".repeat(200_000)), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}

/// A request id full of `"` and `\`: the inbound line already carries it
/// escaped (two bytes per character), and rmcp re-serializes the same
/// escaping into the reply envelope — the id's wire size is doubled. The
/// reply line must still fit.
#[test]
fn mcp_query_reply_line_holds_with_an_escaping_request_id() {
    let fixture = fixture("id-escaping");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let id = format!("{}{}", "\"".repeat(100_000), "\\".repeat(100_000));
    let reply_line = server.call_query(json!(id), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}

/// A non-ASCII request id (100,000 `é`, two UTF-8 bytes each): serde_json
/// carries it through the envelope as raw UTF-8, no `\u` escaping, so the
/// id contributes the same wire bytes as the ASCII case — and the reply
/// line must still fit.
#[test]
fn mcp_query_reply_line_holds_with_a_non_ascii_request_id() {
    let fixture = fixture("id-nonascii");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line = server.call_query(json!("é".repeat(100_000)), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}

/// The engine already marked the result truncated (the connector's 16 MiB
/// result cap stopped it mid-scan), and the reply STILL needed a byte cut:
/// the narrowing note must name the cut — the old note condition compared
/// the truncation flags, which were already equal, and the note went
/// missing. `row_count`, the note's numbers, and the evidence must agree.
#[test]
fn mcp_query_byte_cut_note_appears_when_the_engine_already_truncated() {
    let fixture = fixture("note");
    fixture.seed_blobs_count(20);
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line = server.call_query(json!(3), "SELECT id, payload FROM blobs ORDER BY id");

    assert!(
        reply_line.len() <= MAX_RESPONSE_BYTES,
        "the reply LINE must stay within the {}-byte wire bound, was {} bytes",
        MAX_RESPONSE_BYTES,
        reply_line.len()
    );
    let reply: Value = serde_json::from_str(&reply_line).expect("the reply line is JSON");
    assert_eq!(reply["result"]["isError"], false);
    let payload = &reply["result"]["structuredContent"];
    assert_eq!(
        payload["truncated"], true,
        "the engine already truncated; the byte cut keeps it truncated"
    );
    let note = payload["note"]
        .as_str()
        .expect("the byte cut must be named even when the engine had already truncated");
    // "the response exceeded the byte bound and was cut to {sent}/{total} rows"
    let cut = note
        .split("cut to ")
        .nth(1)
        .expect("the note quantifies the cut: {note}");
    let (sent, total) = cut
        .split_once('/')
        .expect("the note names sent/total rows: {note}");
    let sent: u64 = sent.trim().parse().expect("sent row count in the note");
    let total: u64 = total
        .trim()
        .trim_end_matches(" rows")
        .parse()
        .expect("total row count in the note");
    assert_eq!(
        payload["row_count"]
            .as_u64()
            .expect("row_count is a number"),
        sent,
        "the note's sent count is the payload's row_count: {note}"
    );
    assert!(total > sent, "rows were dropped for the byte bound: {note}");
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

    server.close_and_expect_exit(0);
}

/// The audit's exact case (R29-1): an id of 135,977 ASCII characters with the
/// inline `_meta` that keeps the `resultType` discriminator on the wire. The
/// budget counts the newline and the discriminator, so the full 8-row answer
/// does not fit by exactly one byte: the reply is narrowed — never refused,
/// never sent fat — and stays internally consistent.
#[test]
fn mcp_query_reply_line_holds_with_the_audit_meta_and_boundary_id() {
    let fixture = fixture("id-meta");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line =
        server.call_query_with_meta(json!("i".repeat(135_977)), Some(audit_meta()), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}

/// The same call on the legacy wire shape (no `_meta`): rmcp strips the
/// `resultType` discriminator after the measure, so the reply line is 24
/// bytes SHORTER than the budget — whatever fits the with-form budget fits
/// here with slack to spare.
#[test]
fn mcp_query_reply_line_holds_with_the_audit_id_on_the_legacy_shape() {
    let fixture = fixture("id-legacy");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line = server.call_query(json!("i".repeat(135_977)), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}

/// One probe of the boundary scan: send the audit SQL with an id of `length`
/// ASCII characters and the inline `_meta`, and return the reply line with
/// whether it was narrowed. Every reply must fit the bound and be sent, not
/// refused; a narrowed reply is marked truncated with consistent counts; a
/// full reply carries all 8 rows untruncated.
fn probe(server: &mut TestServer, length: usize) -> (String, bool) {
    let line =
        server.call_query_with_meta(json!("i".repeat(length)), Some(audit_meta()), AUDIT_SQL);
    assert!(
        line.len() <= MAX_RESPONSE_BYTES,
        "the reply LINE must stay within the {}-byte wire bound, was {} bytes \
         (id length {length})",
        MAX_RESPONSE_BYTES,
        line.len()
    );
    let reply: Value = serde_json::from_str(&line).expect("the reply line is JSON");
    assert_eq!(
        reply["result"]["isError"], false,
        "every scanned reply is sent, not refused"
    );
    let payload = &reply["result"]["structuredContent"];
    let sent = payload["row_count"]
        .as_u64()
        .expect("row_count is a number");
    assert_eq!(
        payload["rows"].as_array().expect("rows array").len() as u64,
        sent,
        "row_count matches the rows actually sent (id length {length})"
    );
    let narrowed = sent < 8;
    if narrowed {
        assert_eq!(
            payload["truncated"], true,
            "a narrowed reply is marked truncated (id length {length})"
        );
    } else {
        assert_eq!(
            payload["truncated"], false,
            "a full reply carries all 8 rows untruncated (id length {length})"
        );
    }
    (line, narrowed)
}

/// The exact boundary (R29-1): the reply line is linear in the id length —
/// one byte per ASCII id character rides the envelope — so two full-answer
/// probes below the flip determine the line's constant, and the flip is the
/// smallest id length whose measure tips over the bound. The scan samples id
/// lengths ±64 around that flip: every reply fits the bound, at least one
/// lands within 64 bytes of it — the boundary reply is expected to be
/// EXACTLY the bound — and both sides of the flip appear. One server process
/// carries the whole scan, and the probe count is kept small on purpose:
/// replies are megabytes and the unoptimized test profile makes each
/// round-trip take seconds.
#[test]
fn mcp_query_reply_line_boundary_scan_stays_under_the_cap() {
    let fixture = fixture("boundary-scan");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();

    // Two full-answer probes derive the line's id-length constant; agreeing
    // values confirm the reply line is linear in the id length below the
    // flip. The audit pinned the flip for this SQL and _meta at an id of
    // 135,977, so 100,000 and 101,000 are safely below it.
    let (line_a, narrowed_a) = probe(&mut server, 100_000);
    let (line_b, narrowed_b) = probe(&mut server, 101_000);
    assert!(
        !narrowed_a && !narrowed_b,
        "setup: both derivation probes get the full 8-row answer"
    );
    let constant_a = line_a.len() - 100_000;
    let constant_b = line_b.len() - 101_000;
    assert_eq!(
        constant_a, constant_b,
        "the reply line must be linear in the id length below the flip"
    );
    let flip = MAX_RESPONSE_BYTES - constant_a + 1;

    let mut saw_full = false;
    let mut saw_narrowed = false;
    let mut closest_to_bound = usize::MAX;
    for length in [
        flip.saturating_sub(64),
        flip.saturating_sub(32),
        flip - 1,
        flip,
        flip + 1,
        flip + 64,
    ] {
        let (line, narrowed) = probe(&mut server, length);
        saw_full |= !narrowed;
        saw_narrowed |= narrowed;
        if !narrowed {
            closest_to_bound = closest_to_bound.min(MAX_RESPONSE_BYTES - line.len());
        }
    }
    assert!(
        saw_full && saw_narrowed,
        "the scan must cross the narrowing flip (full: {saw_full}, narrowed: {saw_narrowed})"
    );
    assert!(
        closest_to_bound <= 64,
        "at least one reply must land within 64 bytes of the bound; the \
         closest full reply was {closest_to_bound} bytes below it"
    );

    server.close_and_expect_exit(0);
}
