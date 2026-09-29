//! Shared harness for the deterministic correctness scenarios (B3d).
//!
//! Every scenario builds the real demo fixture with the actual `saya demo`
//! binary (SQLite, fixed seed: customers 240, orders 560, customer_contacts
//! 123), scripts an OpenAI-compatible mock provider round by round, and runs
//! the real `saya ask` binary headlessly. Assertions land on the rows saya
//! returned to the model — read from the tool result the follow-up provider
//! request carries — and on the truth computed independently by a second
//! `saya query` against the same fixture. No network beyond 127.0.0.1; the
//! mock harness is the tests/mvp one, duplicated because tests in different
//! binaries cannot share modules.

use std::{
    fs,
    io::Read,
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

/// An isolated, pid-suffixed scratch root for one scenario.
pub(crate) fn test_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-correctness-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// Reads one HTTP request from the mock's accepted connection to completion:
/// the headers, then exactly the `Content-Length` body bytes — returned as a
/// string so a scenario can assert on what saya sent (the knowledge block, the
/// tool result). The full drain mirrors tests/mvp/common.rs: reqwest sends the
/// body after the headers, and a single short read leaves the socket with
/// unread bytes, so closing it resets the connection mid-request and the
/// client retries, consuming the scripted responses.
pub(crate) fn drain_request(stream: &mut std::net::TcpStream) -> String {
    let (mut buffer, mut chunk) = (Vec::new(), [0_u8; 8192]);
    loop {
        let head = buffer.windows(4).position(|window| window == b"\r\n\r\n");
        if let Some(head) = head {
            let length = String::from_utf8_lossy(&buffer[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if buffer.len() >= head + 4 + length {
                return String::from_utf8_lossy(&buffer).into_owned();
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return String::from_utf8_lossy(&buffer).into_owned(),
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
}

/// A scripted OpenAI-compatible provider. One SSE body per connection, in
/// script order; every request body is captured so a scenario can assert on
/// what saya sent (the knowledge block, the tool result). Serves exactly
/// `responses.len()` connections, then stops: an unexpected extra provider
/// call fails loudly (connection refused) rather than being absorbed.
pub(crate) struct MockProvider {
    address: String,
    bodies: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

pub(crate) fn spawn_mock(responses: Vec<String>) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&bodies);
    let handle = thread::spawn(move || {
        for body in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = drain_request(&mut stream);
            captured.lock().unwrap().push(request);
            write_response(&mut stream, &body);
        }
    });
    MockProvider {
        address,
        bodies,
        handle: Some(handle),
    }
}

fn write_response(stream: &mut std::net::TcpStream, body: &str) {
    use std::io::Write;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    stream.flush().unwrap();
}

impl MockProvider {
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// Captured request bodies, in script order: body[i] is what saya sent on
    /// the round the mock answered with the i-th scripted response. Each
    /// entry is the raw HTTP text (headers + JSON body); use
    /// [`request_json`] to read the JSON part.
    pub(crate) fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }

    pub(crate) fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

/// One SSE `data:` frame carrying a single tool call, ready for the mock to
/// serve. Bodies are built with `serde_json`, never by hand-escaping.
pub(crate) fn tool_call_body(call_id: &str, name: &str, args: serde_json::Value) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&args).unwrap(),
                    },
                }],
            },
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// One SSE `data:` frame carrying a plain-text final answer.
pub(crate) fn text_body(text: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": { "content": text },
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// The isolated fixture a scenario runs: the real demo database plus the
/// config/state roots `saya ask` and `saya query` read.
pub(crate) struct DemoFixture {
    pub root: PathBuf,
    pub connections: PathBuf,
    pub config_home: PathBuf,
    pub state_db: PathBuf,
}

/// Builds the real demo fixture headlessly via SAYA_DEMO_DIR.
pub(crate) fn build_demo(label: &str) -> DemoFixture {
    let root = test_root(label);
    let demo_dir = root.join("demo");
    let config_home = root.join("user-config");
    let state_db = root.join("state.sqlite3");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["demo", "--non-interactive", "--format", "json"])
        .env("SAYA_DEMO_DIR", &demo_dir)
        .env("SAYA_CONFIG_HOME", &config_home)
        .env("SAYA_STATE_DB", &state_db)
        .env("HOME", &root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "demo must build: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let connections = demo_dir.join("connections.toml");
    assert!(connections.exists());
    DemoFixture {
        root,
        connections,
        config_home,
        state_db,
    }
}

/// Runs the real `saya ask` headlessly with the mock as provider.
pub(crate) fn run_ask(
    fixture: &DemoFixture,
    mock_address: &str,
    prompt: &str,
) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "ndjson",
            "--connections",
            fixture.connections.to_str().unwrap(),
            "--profile",
            "demo",
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "ask",
            prompt,
        ])
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{mock_address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &fixture.state_db)
        .env("SAYA_ALLOW_DATA_SHARING", "true")
        .env("HOME", &fixture.root)
        .output()
        .unwrap()
}

/// Direct `saya query`: the independent truth for a scenario.
pub(crate) fn run_query(fixture: &DemoFixture, sql: &str) -> Vec<Vec<serde_json::Value>> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            fixture.connections.to_str().unwrap(),
            "--profile",
            "demo",
            "query",
            "--sql",
            sql,
        ])
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_STATE_DB", &fixture.state_db)
        .env("HOME", &fixture.root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "truth query must succeed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let event = stdout
        .lines()
        .find_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value.get("event")?.as_str() == Some("query_result")).then_some(value)
        })
        .unwrap_or_else(|| panic!("no query_result in: {stdout}"));
    event
        .pointer("/result/rows")
        .and_then(|rows| rows.as_array())
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().clone())
        .collect()
}

/// The JSON body of a captured raw HTTP request (headers + body).
pub(crate) fn request_json(raw: &str) -> serde_json::Value {
    let body = raw.rsplit("\r\n\r\n").next().unwrap_or(raw);
    serde_json::from_str(body).unwrap()
}

/// The rows the model saw: the tool-result message of the follow-up request.
pub(crate) fn tool_result_rows(request_body: &str) -> Vec<Vec<serde_json::Value>> {
    let body = request_json(request_body);
    let messages = body.get("messages").and_then(|m| m.as_array()).unwrap();
    let tool_message = messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .expect("the follow-up request carries the tool result");
    let content = tool_message
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(content).unwrap();
    result
        .get("rows")
        .and_then(|rows| rows.as_array())
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().clone())
        .collect()
}
