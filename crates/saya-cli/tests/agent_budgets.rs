//! Proves the env-driven budget ceilings at the real binary boundary.
//!
//! The ceilings the runtime reads from the process environment
//! (`runtime.rs` composes them onto `AgentLimits`) are proven by spawning the
//! real headless `saya ask` with the ceiling variables planted **on the child
//! `Command` only** — never in this test process, whose parallel tests run
//! the same runtime and must never observe a planted ceiling. The stop point
//! is observed from what the scripted mock provider received (request count,
//! whether each request carried tools) and the child's output/exit.
//!
//! The mock harness is a trimmed copy of `tests/correctness/common.rs`
//! (tests in different binaries cannot share modules), with one adaptation:
//! the thread serves the script, then an unbounded text fallback, until a
//! quiet period with no new connection passes — so a run stopped early by a
//! ceiling still exits and `join` cannot hang.

use std::{
    fs,
    io::{self, Read},
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// The ceiling variables the runtime composes onto `AgentLimits` (the names
/// `budgets_from_env` reads). Scrubbed from every child, then planted per
/// scenario, so no scenario inherits an ambient value.
const BUDGET_VARS: [&str; 3] = [
    "SAYA_AGENT_MAX_TURNS",
    "SAYA_AGENT_MAX_TOOL_CALLS",
    "SAYA_AGENT_MAX_CONTINUATIONS",
];

/// The ask can always answer: after the script the mock serves plain text,
/// so the process exits on its own. Request *shape* is the oracle, not the
/// answer — a swapped or dropped ceiling changes how many scripted rounds
/// the loop consumes, and whether the last request stripped its tools.
const FALLBACK_TEXT: &str = "fallback answer from the script-exhausted mock";

/// The text answer the ceiling scenarios share: it sits where the ceiling
/// salvage call lands, so the planted regime and the dropped-ceiling regime
/// terminate on the SAME words — what differs is the requests' tools
/// signature, never the prose.
const CEILING_TEXT: &str = "the orders count is 560";

/// How long the mock waits for one more connection before returning: long
/// enough that a healthy run's next round arrives, short enough that a
/// budget-stopped run's `join` returns instead of hanging. Reset on every
/// accepted connection.
const QUIET_AFTER_LAST: Duration = Duration::from_secs(8);

/// An isolated, pid-suffixed scratch root for one scenario.
fn test_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-agent-budgets-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// Reads one HTTP request from the mock's accepted connection to completion:
/// the headers, then exactly the `Content-Length` body bytes. Copied from
/// `tests/correctness/common.rs`.
fn drain_request(stream: &mut std::net::TcpStream) -> String {
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

/// A scripted OpenAI-compatible provider: one SSE body per connection in
/// script order (then the text fallback), capturing every request body. The
/// thread returns after a quiet period with no new connection, so `join`
/// never hangs on a run a budget ceiling stopped early.
struct MockProvider {
    address: String,
    bodies: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

fn spawn_mock(responses: Vec<String>) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&bodies);
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let mut script = responses.into_iter();
        let mut last_activity = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    last_activity = Instant::now();
                    stream.set_nonblocking(false).unwrap();
                    let request = drain_request(&mut stream);
                    captured.lock().unwrap().push(request);
                    let body = script.next().unwrap_or_else(|| text_body(FALLBACK_TEXT));
                    write_response(&mut stream, &body);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if last_activity.elapsed() >= QUIET_AFTER_LAST {
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("mock accept failed: {error}"),
            }
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
    fn address(&self) -> &str {
        &self.address
    }

    /// Captured request bodies, in script order.
    fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }

    /// Joins the mock thread; always returns (bounded quiet period).
    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

/// One SSE `data:` frame carrying a single tool call. Bodies are built with
/// `serde_json`, never by hand-escaping.
fn tool_call_body(call_id: &str, name: &str, args: serde_json::Value) -> String {
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
fn text_body(text: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": { "content": text },
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// One SSE `data:` frame carrying the provider's output-token truncation
/// signal: `finish_reason: "length"`, which the OpenAI wire maps to the
/// typed `OutputTruncated` error the continuation ceiling bounds.
fn truncated_body() -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": { "content": "a partial answer that never completes" },
            "finish_reason": "length",
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// The isolated fixture a scenario runs: the real demo database plus the
/// config/state roots `saya ask` reads. Memory stays off (the default), so
/// recall and the post-turn extraction never spend provider calls.
struct DemoFixture {
    root: PathBuf,
    connections: PathBuf,
    config_home: PathBuf,
    state_db: PathBuf,
}

/// Builds the real demo fixture headlessly via SAYA_DEMO_DIR. Copied from
/// `tests/correctness/common.rs`.
fn build_demo(label: &str) -> DemoFixture {
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

/// Runs the real `saya ask` headlessly with the mock as provider and the
/// given budget variables planted on the child only. Every other budget
/// variable is scrubbed from the child so no scenario inherits an ambient
/// value. Copied from `tests/correctness/common.rs`, plus `budgets`.
fn run_ask(
    fixture: &DemoFixture,
    mock_address: &str,
    prompt: &str,
    budgets: &[(&str, &str)],
) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_saya"));
    command
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
        .env("HOME", &fixture.root);
    for name in BUDGET_VARS {
        command.env_remove(name);
    }
    for (name, value) in budgets {
        command.env(name, value);
    }
    command.output().unwrap()
}

/// The JSON body of a captured raw HTTP request (headers + body).
fn request_json(raw: &str) -> serde_json::Value {
    let body = raw.rsplit("\r\n\r\n").next().unwrap_or(raw);
    serde_json::from_str(body).unwrap()
}

/// Whether the captured request advertised tool definitions — the loop's
/// answering turns do; the ceiling salvage call strips them.
fn request_has_tools(raw: &str) -> bool {
    request_json(raw)
        .get("tools")
        .and_then(|tools| tools.as_array())
        .is_some_and(|tools| !tools.is_empty())
}

/// Twelve scripted `bounded_sql_query` rounds against the demo fixture: each
/// returns one tool call on a real table, and the run's own work (plus the
/// always-available text fallback) always terminates it — so a dropped
/// ceiling degrades to a completed run instead of hanging the test.
fn tool_call_script(rounds: usize) -> Vec<String> {
    (0..rounds)
        .map(|round| {
            tool_call_body(
                &format!("call-{round}"),
                "bounded_sql_query",
                serde_json::json!({
                    "sql": "SELECT COUNT(*) FROM orders",
                }),
            )
        })
        .collect()
}

/// Three scripted `bounded_sql_query` rounds followed by a plain-text answer.
/// The text sits where the ceiling salvage call lands: under the planted
/// ceiling the loop stops after three answering turns and salvages this
/// text; a dropped ceiling degrades to a natural completion on the same
/// text instead of hanging. Either way the run terminates with an answer —
/// the ceiling stop is distinguished by the requests' tools signature (the
/// salvage call strips the tool definitions), not by the answer.
fn three_rounds_then_text(answer: &str) -> Vec<String> {
    let mut script = tool_call_script(3);
    script.push(text_body(answer));
    script
}

/// Runs one scenario and returns what the mock received plus the child's
/// output: `(request_count, tools_on_each_request, child_output)`.
fn run_scenario(
    label: &str,
    script: Vec<String>,
    budgets: &[(&str, &str)],
) -> (usize, Vec<bool>, std::process::Output) {
    let fixture = build_demo(label);
    let mut mock = spawn_mock(script);
    let output = run_ask(&fixture, mock.address(), "how many orders exist", budgets);
    mock.join();
    let bodies = mock.bodies();
    let tools = bodies.iter().map(|body| request_has_tools(body)).collect();
    let _ = fs::remove_dir_all(&fixture.root);
    (bodies.len(), tools, output)
}

/// The turn ceiling: `SAYA_AGENT_MAX_TURNS=3` stops the loop after exactly
/// three answering turns plus the ceiling salvage call — proving the
/// env-parsed value is what the loop actually received. The tool-call value
/// (7) is distinct from the turn value (3) so a swap is detectable: with the
/// tool-call value on the turn field the loop would consume more scripted
/// rounds before stopping.
#[test]
fn turn_ceiling_stops_the_loop_at_the_planted_turn_count() {
    let (requests, tools, output) = run_scenario(
        "turn-ceiling",
        three_rounds_then_text(CEILING_TEXT),
        &[
            ("SAYA_AGENT_MAX_TURNS", "3"),
            ("SAYA_AGENT_MAX_TOOL_CALLS", "7"),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a ceiling stop salvages a best answer: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        requests, 4,
        "three answering turns plus the ceiling salvage call (tools={tools:?})",
    );
    assert_eq!(
        tools,
        vec![true, true, true, false],
        "the first three requests carry tools, the salvage call strips them",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(CEILING_TEXT),
        "the salvaged answer is the scripted text: {stdout}",
    );
}

/// The tool-call ceiling: `SAYA_AGENT_MAX_TOOL_CALLS=2` stops the loop when
/// a turn's batch would cross it — after two executed calls and one refused
/// batch, with the salvage call. The turn value (9) is distinct from the
/// tool-call value (2) so a swap is detectable: with the turn value on the
/// tool-call field the loop would run nine answering turns instead of three.
#[test]
fn tool_call_ceiling_stops_the_loop_at_the_planted_call_count() {
    let (requests, tools, output) = run_scenario(
        "tool-call-ceiling",
        three_rounds_then_text(CEILING_TEXT),
        &[
            ("SAYA_AGENT_MAX_TURNS", "9"),
            ("SAYA_AGENT_MAX_TOOL_CALLS", "2"),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a ceiling stop salvages a best answer: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        requests, 4,
        "two calls executed, the third turn's batch refused, plus the salvage \
         call (tools={tools:?})",
    );
    assert_eq!(
        tools,
        vec![true, true, true, false],
        "the first three requests carry tools, the salvage call strips them",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(CEILING_TEXT),
        "the salvaged answer is the scripted text: {stdout}",
    );
}

/// The continuation ceiling: `SAYA_AGENT_MAX_CONTINUATIONS=2` re-instructs
/// the model exactly twice on a deterministic truncation, then surfaces the
/// truncation error. The script serves three truncations (the initial call
/// plus two continuations) and then plain text: the planted ceiling binds
/// first, so the run fails without ever reaching the text.
#[test]
fn continuation_ceiling_bounds_the_truncation_retries() {
    let (requests, _tools, output) = run_scenario(
        "continuation-ceiling",
        vec![truncated_body(), truncated_body(), truncated_body()],
        &[("SAYA_AGENT_MAX_CONTINUATIONS", "2")],
    );
    assert_eq!(
        requests, 3,
        "one initial call plus exactly the planted two continuations",
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "the final truncation must surface as a failure",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("truncated"),
        "the truncation error surfaced, not something else: {stderr}",
    );
}
