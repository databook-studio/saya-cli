//! Stdio contract for `saya mcp serve` (ADR 0008, task Db). The real binary
//! is driven over pipes, like a real MCP client: newline-delimited JSON-RPC
//! on stdout and nothing else, a toolset bounded to the startup allowlist
//! (names + dialects, never paths, hosts, or identities), row-returning tools
//! listed only when data sharing is allowed, the same read-only safety gate
//! the CLI uses, a JSON-RPC error — not a crash — for an oversized request
//! line, and exit 0 on stdin EOF.
//!
//! Config isolation mirrors the sibling CLI tests: `--config`/`--connections`
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

use serde_json::Value;

const PROTOCOL_VERSION: &str = "2025-06-18";

/// An isolated fixture: two sqlite profiles (`local` and `other`) over one
/// seeded database, config naming `local` the default, and isolated state
/// and investigations paths.
struct Fixture {
    root: PathBuf,
    config: PathBuf,
    connections: PathBuf,
    state: PathBuf,
    investigations: PathBuf,
    database: PathBuf,
}

fn crate_fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("saya-cli-mcp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.sqlite3");
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.other]\ntype = 'sqlite'\npath = '{}'\n",
            database.display(),
            root.join("other.sqlite3").display(),
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
    /// Seeds the `events` table through sqlx, directly (the connector is
    /// read-only; only the test writes).
    fn seed_events(&self) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&self.database)
                    .create_if_missing(true);
                let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
                sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
                    .execute(&pool)
                    .await
                    .unwrap();
                sqlx::query("INSERT INTO events (id, label) VALUES (1, 'first'), (2, 'second')")
                    .execute(&pool)
                    .await
                    .unwrap();
                pool.close().await;
            });
    }

    /// Seeds a sentinel table into `other.sqlite3`: if a replay ever reached
    /// this non-allowlisted database, an investigation reading it would
    /// succeed instead of being refused.
    fn seed_sentinel(&self, label: &str) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(self.root.join("other.sqlite3"))
                    .create_if_missing(true);
                let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
                sqlx::query("CREATE TABLE sentinel (label TEXT NOT NULL)")
                    .execute(&pool)
                    .await
                    .unwrap();
                sqlx::query("INSERT INTO sentinel (label) VALUES (?)")
                    .bind(label)
                    .execute(&pool)
                    .await
                    .unwrap();
                pool.close().await;
            });
    }

    /// Runs a CLI subcommand against this fixture with the same isolated
    /// environment the server gets, for the seeding steps the MCP surface
    /// itself never offers.
    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_saya"))
            .args([
                "--non-interactive",
                "--config",
                self.config.to_str().unwrap(),
                "--connections",
                self.connections.to_str().unwrap(),
            ])
            .args(args)
            .env("SAYA_STATE_DB", &self.state)
            .env("SAYA_INVESTIGATIONS_DIR", &self.investigations)
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home").join(".config"))
            .env_remove("SAYA_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Saves one investigation through the CLI and returns its id.
    fn save_investigation(&self, name: &str, sql: &str, connection: &str) -> String {
        let (code, out, err) = self.cli(&[
            "investigation",
            "save",
            "--name",
            name,
            "--sql",
            sql,
            "--connection",
            connection,
        ]);
        assert_eq!(code, 0, "save failed: {out}{err}");
        out.lines()
            .next()
            .expect("the id is the first line")
            .trim()
            .to_string()
    }
}

struct TestServer {
    stdin: Option<ChildStdin>,
    responses: Receiver<String>,
    child: Child,
}

impl TestServer {
    /// Spawn the real binary serving MCP over stdio; `extra` are the
    /// subcommand arguments (e.g. `["mcp", "serve", "--profile", "local"]`).
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

    /// The next stdout line as a JSON-RPC 2.0 value; panics past the deadline
    /// so a hung server fails the test instead of stalling the suite.
    fn next_json(&self, seconds: u64) -> Value {
        let line = self.next_line(seconds);
        let value: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("stdout line is not JSON: {error}: {line}"));
        assert_eq!(
            value["jsonrpc"], "2.0",
            "every stdout line is JSON-RPC 2.0: {line}"
        );
        value
    }

    fn next_line(&self, seconds: u64) -> String {
        match self.responses.recv_timeout(Duration::from_secs(seconds)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                panic!("no server response within {seconds}s");
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("server stdout closed before the expected response");
            }
        }
    }

    /// Whether a response for `id` arrived within the window; scans past any
    /// other frames (their ids are reported back for diagnostics).
    fn got_response_for(&self, id: i64, milliseconds: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(milliseconds);
        while Instant::now() < deadline {
            match self.responses.recv_timeout(Duration::from_millis(
                (deadline - Instant::now()).as_millis() as u64,
            )) {
                Ok(line) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line)
                        && value["id"].as_i64() == Some(id)
                    {
                        return true;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        false
    }

    /// Assert no response for `id` arrives within the window.
    fn expect_no_response_for(&self, id: i64, milliseconds: u64) {
        assert!(
            !self.got_response_for(id, milliseconds),
            "the cancelled call must never be answered (id {id})"
        );
    }

    /// Close stdin and require exit 0 within 5 s (invariant 4).
    fn close_and_expect_exit(&mut self, expected: i32) {
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_secs(5);
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
        // Nothing but consumed responses may sit on stdout.
        if let Ok(line) = self.responses.recv_timeout(Duration::from_millis(200)) {
            panic!("unexpected stdout line after the last response: {line}");
        }
    }

    /// Wait for the child to exit on its own (used for refused startups). The
    /// deadline is generous — this pins the exit code, not the latency, and a
    /// parallel test run can starve this thread's wall clock.
    fn wait_for_exit(&mut self) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                return status.code().expect("exit code");
            }
            assert!(Instant::now() < deadline, "server did not exit within 15s");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn stderr(&mut self) -> String {
        let mut stderr = self.child.stderr.take().expect("piped stderr");
        let mut buffer = String::new();
        std::io::Read::read_to_string(&mut stderr, &mut buffer).unwrap();
        buffer
    }

    fn handshake(&mut self) {
        self.send(&initialize_request());
        let init = self.next_json(10);
        assert_eq!(
            init["result"]["protocolVersion"], PROTOCOL_VERSION,
            "the requested protocol revision is echoed"
        );
        assert_eq!(
            init["result"]["serverInfo"]["name"],
            "saya",
            "initialize names the server: {}",
            serde_json::to_string(&init).unwrap()
        );
        assert_eq!(
            init["result"]["serverInfo"]["version"],
            env!("CARGO_PKG_VERSION"),
            "initialize reports the crate version"
        );
        assert!(
            init["result"]["capabilities"]["tools"].is_object(),
            "initialize advertises the tools capability: {}",
            serde_json::to_string(&init).unwrap()
        );
        self.send(initialized_notification());
    }

    /// `tools/list` tool names, order-independent.
    fn listed_tools(&mut self, id: i64) -> Vec<String> {
        self.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list"}}"#
        ));
        let tools = self.next_json(10);
        assert_eq!(tools["id"], id);
        tools["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_string())
            .collect()
    }

    /// One tools/call round-trip; asserts the frame is JSON-RPC and returns it.
    fn call(&mut self, id: i64, name: &str, arguments: &str) -> Value {
        self.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#
        ));
        let call = self.next_json(30);
        assert_eq!(call["id"], id);
        call
    }

    /// One tools/call expected to answer as a tool-level error; returns the
    /// flattened text the client would show.
    fn call_error_text(&mut self, id: i64, name: &str, arguments: &str) -> String {
        let call = self.call(id, name, arguments);
        assert_eq!(
            call["result"]["isError"],
            true,
            "expected isError: {}",
            serde_json::to_string(&call).unwrap()
        );
        call["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}

fn initialize_request() -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{PROTOCOL_VERSION}","capabilities":{{}},"clientInfo":{{"name":"contract-test","version":"0"}}}}}}"#
    )
}

fn initialized_notification() -> &'static str {
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
}

const SLOW_SQL: &str = "WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c) \
                        SELECT sum(x) FROM c";

/// The full happy path with data sharing on: initialize negotiates, every
/// advertised tool answers, every stdout line is a JSON-RPC frame, an
/// oversized line gets -32600 without taking the server down, and stdin EOF
/// exits 0.
#[test]
fn mcp_stdout_contains_only_protocol_frames() {
    let fixture = crate_fixture("frames");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &[
            "mcp",
            "serve",
            "--profile",
            "local",
            "--profile",
            "other",
            "--allow-data-sharing",
        ],
    );

    server.handshake();

    // tools/list → the shared toolset, all described and schematized.
    server.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let tools = server.next_json(10);
    assert_eq!(tools["id"], 2);
    let list = tools["result"]["tools"].as_array().expect("tools array");
    let names: Vec<_> = list
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert_eq!(
        names,
        vec![
            "contracts",
            "investigation_run",
            "list_profiles",
            "query",
            "schema"
        ],
        "the shared toolset is advertised in the documented order"
    );
    for tool in list {
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "every tool describes itself: {tool}"
        );
        assert!(tool["inputSchema"].is_object(), "every tool has a schema");
    }
    let replay_tool = list
        .iter()
        .find(|tool| tool["name"] == "investigation_run")
        .expect("the replay tool is advertised");
    assert_eq!(
        replay_tool["inputSchema"]["properties"]["params"]["type"], "object",
        "the replay tool advertises its params map: {replay_tool}"
    );
    assert_eq!(
        replay_tool["inputSchema"]["properties"]["params"]["additionalProperties"]["type"],
        "string",
        "params values are strings, parsed by the typed parser: {replay_tool}"
    );

    // list_profiles → names and dialects only.
    let profiles = server.call(3, "list_profiles", "{}");
    assert_eq!(profiles["result"]["isError"], false);
    assert_eq!(
        profiles["result"]["structuredContent"]["profiles"],
        serde_json::json!([
            {"name": "local", "dialect": "sqlite"},
            {"name": "other", "dialect": "sqlite"}
        ]),
    );

    // schema → the compact tree for the allowed profile.
    let schema = server.call(4, "schema", r#"{"profile":"local"}"#);
    assert_eq!(schema["result"]["isError"], false);
    assert_eq!(schema["result"]["structuredContent"]["profile"], "local");
    let tables = schema["result"]["structuredContent"]["tables"]
        .as_object()
        .expect("compact schema tables");
    assert!(
        tables.contains_key("events"),
        "the seeded table is in the compact tree: {tables:?}"
    );

    // query → bounded read-only rows with mcp-sourced evidence.
    let query = server.call(
        5,
        "query",
        r#"{"profile":"local","sql":"SELECT id, label FROM events ORDER BY id"}"#,
    );
    assert_eq!(query["result"]["isError"], false);
    let payload = &query["result"]["structuredContent"];
    assert_eq!(payload["columns"], serde_json::json!(["id", "label"]));
    assert_eq!(payload["row_count"], 2);
    assert_eq!(payload["truncated"], false);
    assert!(
        payload["source"].is_null(),
        "the source is named by the typed evidence, not an ad-hoc marker: {payload}"
    );
    assert_eq!(
        payload["evidence"]["source"]["kind"], "mcp",
        "the query evidence names this server as its source: {payload}"
    );
    assert!(
        payload["evidence"]["execution_id"].is_string(),
        "the result carries execution evidence: {payload}"
    );
    let rendered = serde_json::to_string(&query).unwrap();
    assert!(
        !rendered.contains("data.sqlite3"),
        "the sqlite path never reaches the client: {rendered}"
    );

    // contracts → the active claim (no identities). The schema cache is
    // refreshed first, then one confirmed claim is remembered through the
    // CLI; both tool forms answer with it.
    let (code, out, err) = fixture.cli(&["connection", "schema", "local", "--refresh"]);
    assert_eq!(code, 0, "schema refresh failed: {out}{err}");
    let (code, out, err) = fixture.cli(&[
        "contracts",
        "remember",
        "data.main.events",
        "--kind",
        "description",
        "--value",
        "orders header table",
        "--profile",
        "local",
    ]);
    assert_eq!(code, 0, "remember failed: {out}{err}");
    let contracts = server.call(6, "contracts", r#"{"profile":"local"}"#);
    assert_eq!(contracts["result"]["isError"], false);
    let listed = contracts["result"]["structuredContent"]["contracts"]
        .as_array()
        .expect("contracts array");
    assert_eq!(listed.len(), 1, "the confirmed claim is listed: {listed:?}");
    assert_eq!(listed[0]["object"], "data.main.events");
    assert_eq!(listed[0]["claims"][0]["status"], "confirmed");
    let one = server.call(
        7,
        "contracts",
        r#"{"profile":"local","table":"data.main.events"}"#,
    );
    assert_eq!(one["result"]["isError"], false);
    assert_eq!(
        one["result"]["structuredContent"]["claims"][0]["value"],
        "orders header table"
    );
    let rendered = serde_json::to_string(&one).unwrap();
    assert!(
        !rendered.contains("p-") && !rendered.contains("identity"),
        "the opaque profile identity never reaches the client: {rendered}"
    );

    // An unknown tool is a JSON-RPC method error, not a crash.
    server.send(
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"no_such_tool","arguments":{}}}"#,
    );
    let unknown = server.next_json(10);
    assert_eq!(unknown["id"], 9);
    assert_eq!(
        unknown["error"]["code"], -32601,
        "unknown tool: method not found"
    );

    // An oversized line is answered with -32600 and the server keeps serving.
    let oversized = format!(
        r#"{{"jsonrpc":"2.0","id":7,"method":"mcp/padded","params":{{"pad":"{}"}}}}"#,
        "A".repeat(1024 * 1024 + 64),
    );
    assert!(oversized.len() > 1024 * 1024);
    server.send(&oversized);
    let rejected = server.next_json(10);
    assert_eq!(
        rejected["error"]["code"], -32600,
        "oversized line: {rejected}"
    );
    assert_eq!(rejected["error"]["message"], "request too large");
    assert!(
        rejected["id"].is_null(),
        "a discarded line is never parsed, so its id cannot be echoed: {rejected}"
    );

    // The server is still alive after the oversized line.
    server.send(r#"{"jsonrpc":"2.0","id":6,"method":"ping"}"#);
    let ping = server.next_json(5);
    assert_eq!(ping["id"], 6);
    assert!(ping["result"].is_object());

    server.close_and_expect_exit(0);
}

/// Without `--profile`, the allowlist is the configured default profile.
#[test]
fn mcp_stdio_list_profiles_uses_the_configured_default() {
    let fixture = crate_fixture("default");
    let mut server = TestServer::spawn(&fixture, &["mcp", "serve"]);

    server.handshake();
    let call = server.call(5, "list_profiles", "{}");
    assert_eq!(
        call["result"]["structuredContent"]["profiles"],
        serde_json::json!([{"name": "local", "dialect": "sqlite"}]),
        "the configured default profile forms the allowlist"
    );

    server.close_and_expect_exit(0);
}

/// A `--profile` naming nothing in the resolved connections is refused at
/// startup with a usage error, before any protocol byte is written.
#[test]
fn mcp_stdio_unknown_profile_is_refused_at_startup() {
    let fixture = crate_fixture("unknown");
    let mut server = TestServer::spawn(&fixture, &["mcp", "serve", "--profile", "missing"]);

    let code = server.wait_for_exit();
    assert_eq!(code, 2, "unknown profile is a usage error");
    if let Ok(line) = server.responses.recv_timeout(Duration::from_millis(300)) {
        panic!("no protocol bytes on a refused startup, got: {line}");
    }
    let stderr = server.stderr();
    assert!(
        stderr.contains("missing"),
        "the refusal names the profile: {stderr}"
    );
}

/// The client cannot widen the startup allowlist: a configured profile that
/// was not allowlisted is "profile not available" on every tool, and
/// `list_profiles` never reveals it.
#[test]
fn mcp_client_cannot_expand_profile_allowlist() {
    let fixture = crate_fixture("allowlist");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let profiles = server.call(2, "list_profiles", "{}");
    assert_eq!(
        profiles["result"]["structuredContent"]["profiles"],
        serde_json::json!([{"name": "local", "dialect": "sqlite"}]),
        "only the allowlisted profile is listed, though 'other' is configured"
    );

    for (id, name, arguments) in [
        (3, "schema", r#"{"profile":"other"}"#),
        (4, "query", r#"{"profile":"other","sql":"SELECT 1"}"#),
        (5, "contracts", r#"{"profile":"other"}"#),
    ] {
        let text = server.call_error_text(id, name, arguments);
        assert!(
            text.contains("profile not available"),
            "{name} refuses a non-allowlisted profile: {text}"
        );
    }

    server.close_and_expect_exit(0);
}

/// `query` rides the same safety gate the CLI uses: a DELETE is an isError
/// with the read-only message, and a SELECT returns rows.
#[test]
fn mcp_query_uses_same_safety_path() {
    let fixture = crate_fixture("safety");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let text = server.call_error_text(
        2,
        "query",
        r#"{"profile":"local","sql":"DELETE FROM events"}"#,
    );
    assert!(
        text.contains("read-only safety policy") && text.contains("DELETE"),
        "the refusal is the safety layer's own words: {text}"
    );

    let query = server.call(
        3,
        "query",
        r#"{"profile":"local","sql":"SELECT count(*) AS n FROM events"}"#,
    );
    assert_eq!(query["result"]["isError"], false);
    assert_eq!(query["result"]["structuredContent"]["row_count"], 1);

    // The written rows are untouched: the gate runs before execution.
    let (code, out, err) = fixture.cli(&["query", "--sql", "SELECT count(*) AS n FROM events"]);
    assert_eq!(
        code, 0,
        "the database is intact after the refused delete: {out}{err}"
    );

    server.close_and_expect_exit(0);
}

/// Data sharing off (the CLI's own fold): row-returning tools are not listed
/// and are refused if called anyway; config-on and `--no-data-sharing` fold
/// exactly as the CLI folds them.
#[test]
fn mcp_denial_matches_cli_policy() {
    // (a) default: sharing off — row tools absent and refused.
    let fixture = crate_fixture("denial-off");
    fixture.seed_events();
    let mut server = TestServer::spawn(&fixture, &["mcp", "serve", "--profile", "local"]);
    server.handshake();
    let listed = server.listed_tools(2);
    assert!(
        !listed.contains(&"query".to_string())
            && !listed.contains(&"investigation_run".to_string()),
        "row tools are not listed with sharing off: {listed:?}"
    );
    assert!(listed.contains(&"schema".to_string()));
    assert!(listed.contains(&"contracts".to_string()));
    let text = server.call_error_text(3, "query", r#"{"profile":"local","sql":"SELECT 1"}"#);
    assert!(
        text.contains("data sharing"),
        "the refusal names the gate: {text}"
    );
    let replay =
        server.call_error_text(4, "investigation_run", r#"{"id":"no-such-investigation"}"#);
    assert!(
        replay.contains("data sharing"),
        "investigation_run is refused the same way: {replay}"
    );
    server.close_and_expect_exit(0);

    // (b) the user config allows — row tools are listed. The setting lives in
    // the user layer (the trusted one): an explicit `--config` file loads as
    // the project layer, where security-critical settings are reverted.
    let scenario = crate_fixture("denial-config-on");
    let user_config = scenario.root.join("home/.config/saya/config.toml");
    std::fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    std::fs::write(&user_config, "[ai]\nallow_data_sharing = true\n").unwrap();
    let mut server = TestServer::spawn(&scenario, &["mcp", "serve", "--profile", "local"]);
    server.handshake();
    let listed = server.listed_tools(2);
    assert!(
        listed.contains(&"query".to_string()) && listed.contains(&"investigation_run".to_string()),
        "config-on lists the row tools: {listed:?}"
    );
    server.close_and_expect_exit(0);

    // (c) the user config allows but `--no-data-sharing` overrides — off
    // again, the same fold the CLI applies.
    let scenario = crate_fixture("denial-cli-override");
    let user_config = scenario.root.join("home/.config/saya/config.toml");
    std::fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    std::fs::write(&user_config, "[ai]\nallow_data_sharing = true\n").unwrap();
    let mut server = TestServer::spawn(
        &scenario,
        &["mcp", "serve", "--profile", "local", "--no-data-sharing"],
    );
    server.handshake();
    let listed = server.listed_tools(2);
    assert!(
        !listed.contains(&"query".to_string()),
        "--no-data-sharing overrides config: {listed:?}"
    );
    server.close_and_expect_exit(0);
}

/// Concurrency and cancellation are bounded: the fifth of five concurrent
/// slow calls is refused as busy, a cancelled in-flight call is never
/// answered, and the server keeps serving after both.
#[test]
fn mcp_cancellation_and_concurrency_are_bounded() {
    let fixture = crate_fixture("bounds");
    fixture.seed_events();
    let mut config = std::fs::read_to_string(&fixture.config).unwrap();
    config.push_str("\n[run]\nquery_timeout_seconds = 4\n");
    std::fs::write(&fixture.config, config).unwrap();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();

    // Five concurrent slow calls: exactly one is refused as busy, the rest
    // run (and each ends within the connector's own query timeout).
    for id in 10..=14 {
        server.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"query","arguments":{{"profile":"local","sql":"{SLOW_SQL}"}}}}}}"#
        ));
    }
    let mut busy = 0;
    for _ in 0..4 {
        let response = server.next_json(60);
        assert!(
            response["error"].is_null() && response["result"]["isError"] == true,
            "every concurrent outcome is a tool-level result: {response}"
        );
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if text.contains("in-flight") {
            busy += 1;
        }
    }
    assert_eq!(
        busy, 1,
        "exactly one of five concurrent calls is refused busy"
    );

    // A cancelled in-flight call gets no response.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{{"name":"query","arguments":{{"profile":"local","sql":"{SLOW_SQL}"}}}}}}"#
    ));
    std::thread::sleep(Duration::from_millis(400));
    server.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":20,"reason":"client moved on"}}"#);
    server.expect_no_response_for(20, 2500);

    // The server keeps serving.
    server.send(r#"{"jsonrpc":"2.0","id":21,"method":"ping"}"#);
    let ping = server.next_json(10);
    assert_eq!(ping["id"], 21);
    assert!(ping["result"].is_object());

    server.close_and_expect_exit(0);
}

/// `investigation_run` replays through the same typed run operation as
/// `saya investigation run`: a fresh review runs and returns the result and
/// evidence; a stale review is refused with the CLI's own message and is
/// never revalidated.
#[test]
fn mcp_investigation_run_happy_and_stale() {
    let fixture = crate_fixture("replay");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    // Save through the CLI: the review binding is recorded locally.
    let (code, out, err) = fixture.cli(&[
        "investigation",
        "save",
        "--name",
        "event count",
        "--sql",
        "SELECT count(*) AS n FROM events",
        "--connection",
        "local",
    ]);
    assert_eq!(code, 0, "save failed: {out}{err}");
    let id = out
        .lines()
        .next()
        .expect("the id is the first line")
        .trim()
        .to_string();

    server.handshake();

    // Happy path: the replay returns rows and evidence.
    let run = server.call(
        2,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local"}}"#),
    );
    assert_eq!(run["result"]["isError"], false);
    let payload = &run["result"]["structuredContent"];
    assert_eq!(payload["result"]["row_count"], 1);
    assert_eq!(payload["connection"], "local");
    assert!(
        payload["evidence"]["execution_id"].is_string(),
        "the replay carries evidence: {payload}"
    );

    // Stale path: an edit moves the revision; the replay is refused with the
    // CLI's own stale message and is never revalidated.
    let (code, out, err) = fixture.cli(&["investigation", "edit", &id, "--sql", "SELECT 2 AS n"]);
    assert_eq!(code, 0, "edit failed: {out}{err}");
    let text = server.call_error_text(
        3,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local"}}"#),
    );
    assert!(
        text.contains("review is stale"),
        "the stale refusal is the CLI's message: {text}"
    );
    assert!(
        !text.contains("revalidated") || text.contains("--revalidate"),
        "the refusal points at --revalidate instead of doing it: {text}"
    );

    // The binding was not rewritten by the MCP call: the CLI run still needs
    // --revalidate.
    let (code, out, err) = fixture.cli(&["investigation", "run", &id]);
    assert_ne!(code, 0, "MCP must not have revalidated: {out}{err}");

    server.close_and_expect_exit(0);
}

/// `investigation_run` binds declared parameters: `params` maps names to
/// string values, parsed by the same typed parser the CLI uses; a missing
/// required parameter is an isError listing the names and types, and no
/// error ever echoes a supplied value.
#[test]
fn mcp_investigation_run_binds_parameters() {
    const SENTINEL: &str = "HVNS3NT1NEL42";
    let fixture = crate_fixture("replay-params");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    // Save through the CLI with a required integer parameter.
    let (code, out, err) = fixture.cli(&[
        "investigation",
        "save",
        "--name",
        "events up to",
        "--sql",
        "SELECT id, label FROM events WHERE id <= :max_id ORDER BY id",
        "--param-spec",
        "max_id:integer:required",
        "--connection",
        "local",
    ]);
    assert_eq!(code, 0, "save failed: {out}{err}");
    let id = out
        .lines()
        .next()
        .expect("the id is the first line")
        .trim()
        .to_string();

    server.handshake();

    // Happy path: params bind as typed values and the result is filtered;
    // the replay keeps its saved-investigation evidence source and records
    // the bound parameters' names.
    let run = server.call(
        2,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local","params":{{"max_id":"1"}}}}"#),
    );
    assert_eq!(
        run["result"]["isError"],
        false,
        "{}",
        serde_json::to_string(&run).unwrap()
    );
    let payload = &run["result"]["structuredContent"];
    assert_eq!(
        payload["result"]["rows"],
        serde_json::json!([[1, "first"]]),
        "the bound parameter filtered the replay: {}",
        serde_json::to_string(payload).unwrap()
    );
    assert_eq!(payload["evidence"]["source"]["kind"], "saved_investigation");
    assert_eq!(
        payload["evidence"]["param_names"],
        serde_json::json!(["max_id"])
    );

    // Missing required parameter: isError with the names and types.
    let text = server.call_error_text(
        3,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local"}}"#),
    );
    assert!(
        text.contains("missing required parameter(s)")
            && text.contains("max_id")
            && text.contains("integer"),
        "the refusal lists the required names with types: {text}"
    );

    // A malformed value is the typed parser's refusal, and the value itself
    // is never echoed — not in this refusal, and not for an unknown name.
    let text = server.call_error_text(
        4,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local","params":{{"max_id":"{SENTINEL}"}}}}"#),
    );
    assert!(
        text.contains("max_id") && text.contains("not a valid integer"),
        "the refusal is the typed parser's: {text}"
    );
    assert!(
        !text.contains(SENTINEL),
        "the value is never echoed: {text}"
    );

    let text = server.call_error_text(
        5,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local","params":{{"nope":"{SENTINEL}"}}}}"#),
    );
    assert!(
        text.contains("no parameter named"),
        "the refusal names the undeclared parameter: {text}"
    );
    assert!(
        !text.contains(SENTINEL),
        "the value is never echoed: {text}"
    );

    // A non-string value in the params map is a JSON-RPC parameter error,
    // and the server keeps serving.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{"name":"investigation_run","arguments":{{"id":"{id}","profile":"local","params":{{"max_id":1}}}}}}}}"#
    ));
    let refused = server.next_json(10);
    assert_eq!(refused["id"], 6);
    assert_eq!(
        refused["error"]["code"], -32602,
        "a non-string param value is a parameter error: {refused}"
    );
    server.send(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#);
    let ping = server.next_json(5);
    assert_eq!(ping["id"], 7);
    assert!(ping["result"].is_object());

    server.close_and_expect_exit(0);
}

/// The audit reproduction (A922-1, D1): while one slow replay holds the
/// replay slot, a second replay is queued whose binding the ordinary CLI
/// then remaps (`investigation run <id> --connection other --revalidate`).
/// When the slot frees, the queued call must never execute what the binding
/// now names: the target is resolved once inside the serialized section and
/// refused there. The old flow gated the binding before the wait and passed
/// the target through, so the remapped profile executed and returned the
/// other database's rows.
#[test]
fn mcp_investigation_run_replay_binds_one_authorized_target() {
    const SENTINEL: &str = "OTHER-SYNTHETIC-SENTINEL-9";
    let fixture = crate_fixture("replay-remap");
    fixture.seed_events();
    fixture.seed_sentinel(SENTINEL);
    let mut config = std::fs::read_to_string(&fixture.config).unwrap();
    config.push_str("\n[run]\nquery_timeout_seconds = 6\n");
    std::fs::write(&fixture.config, config).unwrap();
    // Both investigations are saved against `local` (save never connects,
    // so the sentinel read saves cleanly against the wrong database); the
    // slow one holds the replay slot, the other is the queued call.
    let slow_id = fixture.save_investigation("slow sum", SLOW_SQL, "local");
    let remap_id =
        fixture.save_investigation("sentinel read", "SELECT label FROM sentinel", "local");

    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );
    server.handshake();

    // (1) The slow replay is in flight and holds the replay slot.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"investigation_run","arguments":{{"id":"{slow_id}"}}}}}}"#
    ));
    // (2) The queued replay arrives while the slot is held; its binding
    // still names `local`, so anything read at admission time passes.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"investigation_run","arguments":{{"id":"{remap_id}"}}}}}}"#
    ));
    // (3) The ordinary CLI remaps the queued investigation to `other` — the
    // exact sequence the audit drove — and finishes long before the slow
    // replay's query timeout frees the slot.
    let (code, out, err) = fixture.cli(&[
        "investigation",
        "run",
        &remap_id,
        "--connection",
        "other",
        "--revalidate",
    ]);
    assert_eq!(code, 0, "the CLI rebind must succeed: {out}{err}");

    // (4) The slow replay times out first; its answer leaves the wire
    // before the queued call's.
    let first = server.next_json(60);
    assert_eq!(first["id"], 2);

    // (5) The queued call resolves inside the serialized section; the
    // binding now names a profile outside the allowlist: refused there,
    // never executed — the sentinel row never leaves its database.
    let second = server.next_json(60);
    assert_eq!(second["id"], 3);
    assert_eq!(
        second["result"]["isError"],
        true,
        "the remapped binding is refused: {}",
        serde_json::to_string(&second).unwrap()
    );
    let text = second["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        text.contains("profile not available"),
        "the refusal names the gate: {text}"
    );
    assert!(
        !text.contains(SENTINEL),
        "the non-allowlisted database is never read: {text}"
    );
    assert!(
        !text.contains("other"),
        "the binding's profile is not echoed: {text}"
    );

    server.close_and_expect_exit(0);
}

/// The state DB's audited profile identities, read through the store's own
/// API (the pool opens and migrates lazily, so a call before any audit is
/// empty, not a missing table). Every replay execution writes one audit row
/// — success or failure — before the answer returns, so an empty list after
/// a call means nothing executed against any profile.
fn audit_profiles(state: &std::path::Path) -> Vec<String> {
    use saya_store::{AuditStore, SqliteStateStore};
    let store = SqliteStateStore::new(state.to_path_buf());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { store.recent_audit(100).await.unwrap() })
        .into_iter()
        .map(|record| record.event.profile_id)
        .collect()
}

/// `investigation_run` never reaches a profile outside the startup
/// allowlist (F-1) — neither through the `profile` argument nor through a
/// saved binding's profile. The refusal happens before anything runs: the
/// non-allowlisted database keeps its sentinel to itself and no audit row
/// exists anywhere. A binding inside the allowlist still runs.
#[test]
fn mcp_investigation_run_stays_inside_the_profile_allowlist() {
    const SENTINEL: &str = "OTHER-ONLY-SENTINEL-7";
    let fixture = crate_fixture("replay-allowlist");
    fixture.seed_events();
    fixture.seed_sentinel(SENTINEL);
    let local_id =
        fixture.save_investigation("local count", "SELECT count(*) AS n FROM events", "local");
    let other_id = fixture.save_investigation("other count", "SELECT label FROM sentinel", "other");

    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );
    server.handshake();
    assert!(
        audit_profiles(&fixture.state).is_empty(),
        "nothing has audited yet"
    );

    // (a) The binding points at `other`, which the server does not serve:
    // refused before anything runs — no audit row exists anywhere, and
    // neither the binding's profile name nor the sentinel value is echoed.
    let text = server.call_error_text(2, "investigation_run", &format!(r#"{{"id":"{other_id}"}}"#));
    assert!(
        text.contains("profile not available"),
        "the non-allowlisted binding is refused: {text}"
    );
    assert!(
        !text.contains("other"),
        "the binding's profile name is not echoed: {text}"
    );
    assert!(
        !text.contains(SENTINEL),
        "the non-allowlisted database is never read: {text}"
    );
    assert!(
        audit_profiles(&fixture.state).is_empty(),
        "a refusal writes no audit row: {:?}",
        audit_profiles(&fixture.state)
    );

    // (b) An explicit `profile` argument naming the non-allowlisted profile
    // is refused the same way — even though this investigation's binding is
    // allowlisted.
    let text = server.call_error_text(
        3,
        "investigation_run",
        &format!(r#"{{"id":"{local_id}","profile":"other"}}"#),
    );
    assert!(
        text.contains("profile not available") && text.contains("other"),
        "the requested name is echoed: {text}"
    );
    assert!(
        audit_profiles(&fixture.state).is_empty(),
        "a refusal writes no audit row: {:?}",
        audit_profiles(&fixture.state)
    );

    // (c) A binding inside the allowlist runs.
    let run = server.call(4, "investigation_run", &format!(r#"{{"id":"{local_id}"}}"#));
    assert_eq!(
        run["result"]["isError"],
        false,
        "{}",
        serde_json::to_string(&run).unwrap()
    );
    assert_eq!(run["result"]["structuredContent"]["connection"], "local");
    let audited = audit_profiles(&fixture.state);
    assert!(
        !audited.is_empty(),
        "the allowed replay audited: {audited:?}"
    );

    server.close_and_expect_exit(0);
}
