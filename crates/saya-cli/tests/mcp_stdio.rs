//! Stdio contract for `saya mcp serve` (ADR 0008, task Da). The real binary
//! is driven over pipes, like a real MCP client: newline-delimited JSON-RPC
//! on stdout and nothing else, a `list_profiles`-only toolset bounded to the
//! startup allowlist (names + dialects, never paths, hosts, or identities),
//! a JSON-RPC error — not a crash — for an oversized request line, and exit 0
//! on stdin EOF.
//!
//! Config isolation mirrors the sibling CLI tests: `--config`/`--connections`
//! point at a temp fixture, so no machine-level profile leaks into the
//! assertions.

use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use serde_json::Value;

const PROTOCOL_VERSION: &str = "2025-06-18";

/// An isolated connections/config fixture with one sqlite profile named
/// `local`, and the config naming it the default. The database file is never
/// opened by these tests: `list_profiles` reports names and dialects only.
fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("saya-cli-mcp-{tag}-{}", std::process::id()));
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
    (config, connections)
}

struct TestServer {
    stdin: Option<ChildStdin>,
    responses: Receiver<String>,
    child: Child,
}

impl TestServer {
    /// Spawn the real binary serving MCP over stdio; `extra` are the
    /// subcommand arguments (e.g. `["mcp", "serve", "--profile", "local"]`).
    fn spawn(config: &Path, connections: &Path, extra: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_saya"))
            .args([
                "--config",
                config.to_str().unwrap(),
                "--connections",
                connections.to_str().unwrap(),
            ])
            .args(extra)
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
        let line = match self.responses.recv_timeout(Duration::from_secs(seconds)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                panic!("no server response within {seconds}s");
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("server stdout closed before the expected response");
            }
        };
        let value: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("stdout line is not JSON: {error}: {line}"));
        assert_eq!(
            value["jsonrpc"], "2.0",
            "every stdout line is JSON-RPC 2.0: {line}"
        );
        value
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
}

fn initialize_request() -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{PROTOCOL_VERSION}","capabilities":{{}},"clientInfo":{{"name":"contract-test","version":"0"}}}}}}"#
    )
}

fn initialized_notification() -> &'static str {
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
}

/// The full happy path: initialize negotiates, the toolset is
/// `list_profiles`-only, the tool answers with allowlist names and dialects
/// only, an unknown tool and an oversized line each get a JSON-RPC error
/// without taking the server down, and stdin EOF exits 0.
#[test]
fn mcp_stdio_serves_jsonrpc_and_exits_on_eof() {
    let (config, connections) = fixture("serve");
    let mut server = TestServer::spawn(
        &config,
        &connections,
        &["mcp", "serve", "--profile", "local"],
    );

    // initialize → server identity and tools capability.
    server.send(&initialize_request());
    let init = server.next_json(10);
    assert_eq!(init["id"], 1);
    assert_eq!(
        init["result"]["protocolVersion"], PROTOCOL_VERSION,
        "the requested protocol revision is echoed"
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "saya");
    assert_eq!(
        init["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "the server advertises tools"
    );

    // notifications/initialized draws no response.
    server.send(initialized_notification());

    // tools/list → exactly one tool, list_profiles.
    server.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let tools = server.next_json(5);
    assert_eq!(tools["id"], 2);
    let list = tools["result"]["tools"].as_array().expect("tools array");
    assert_eq!(list.len(), 1, "the skeleton advertises one tool");
    assert_eq!(list[0]["name"], "list_profiles");
    assert!(
        list[0]["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "the tool describes itself"
    );
    assert!(list[0]["inputSchema"].is_object());

    // tools/call list_profiles → allowlist names + dialects, nothing else.
    server.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_profiles","arguments":{}}}"#,
    );
    let call = server.next_json(5);
    assert_eq!(call["id"], 3);
    assert_eq!(call["result"]["isError"], false);
    assert_eq!(
        call["result"]["structuredContent"]["profiles"],
        serde_json::json!([{"name": "local", "dialect": "sqlite"}]),
        "allowlist names and dialects only"
    );
    let rendered = serde_json::to_string(&call).unwrap();
    assert!(
        !rendered.contains("data.sqlite3") && !rendered.contains("path"),
        "profiles never leak paths or identities: {rendered}"
    );

    // An unknown tool is a JSON-RPC method error, not a crash.
    server.send(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"query","arguments":{}}}"#,
    );
    let unknown = server.next_json(5);
    assert_eq!(unknown["id"], 4);
    assert_eq!(
        unknown["error"]["code"], -32601,
        "unknown tool: method not found"
    );

    // A >1 MiB request line gets a JSON-RPC error, not a crash.
    let oversized = format!(
        r#"{{"jsonrpc":"2.0","id":7,"method":"mcp/padded","params":{{"pad":"{}"}}}}"#,
        "A".repeat(1024 * 1024 + 64),
    );
    assert!(oversized.len() > 1024 * 1024);
    server.send(&oversized);
    let rejected = server.next_json(5);
    assert_eq!(rejected["id"], 7);
    assert!(
        rejected["error"].is_object(),
        "the oversized line errors: {rejected}"
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
    let (config, connections) = fixture("default");
    let mut server = TestServer::spawn(&config, &connections, &["mcp", "serve"]);

    server.send(&initialize_request());
    server.next_json(10);
    server.send(initialized_notification());

    server.send(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_profiles","arguments":{}}}"#,
    );
    let call = server.next_json(5);
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
    let (config, connections) = fixture("unknown");
    let mut server = TestServer::spawn(
        &config,
        &connections,
        &["mcp", "serve", "--profile", "missing"],
    );

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
