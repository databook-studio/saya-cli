//! The one `--tools` list at the real binary boundary, on the `ask` path.
//!
//! `ask` is the narrowest agent surface: it composes the database tools and
//! the clarification tool, and nothing else. So it shows both halves of the
//! rule at once — a name the list holds narrows the surface (`request_
//! clarification` goes away when it is not named), and a connection-tied name
//! is refused rather than believed, because those follow the selected profile.
//!
//! The oracle is what the provider actually received: the tool definitions in
//! the first request body the mock served, not the composition the test
//! believes it built.

use std::{
    fs,
    io::{self, Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// The mock's fallback answer: the run always terminates on plain text.
const FALLBACK: &str = "the ask path answered";

/// How long the mock waits for one more connection before returning.
const QUIET_AFTER_LAST: Duration = Duration::from_secs(8);

/// An isolated, pid-suffixed scratch root for one scenario.
fn test_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-tools-allow-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// The real demo database plus the config/state roots `saya ask` reads.
struct DemoFixture {
    root: PathBuf,
    connections: PathBuf,
    config_home: PathBuf,
    state_db: PathBuf,
}

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

/// Runs the real headless `saya ask` with the mock as provider. `stated` is
/// the `--tools` argument list, empty for none.
fn run_ask(fixture: &DemoFixture, mock_address: &str, stated: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_saya"));
    command.args(["--non-interactive", "--format", "ndjson"]);
    if !stated.is_empty() {
        command.arg("--tools").arg(stated.join(","));
    }
    command
        .args([
            "--connections",
            fixture.connections.to_str().unwrap(),
            "--profile",
            "demo",
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "ask",
            "how many orders exist",
        ])
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{mock_address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &fixture.state_db)
        .env("SAYA_ALLOW_DATA_SHARING", "true")
        .env("HOME", &fixture.root);
    command.output().unwrap()
}

/// The tool names the first captured request advertised, sorted.
fn advertised(bodies: &[String]) -> Vec<String> {
    let body = bodies.first().expect("the ask path requested the provider");
    let body: serde_json::Value =
        serde_json::from_str(body.rsplit("\r\n\r\n").next().unwrap_or(body))
            .expect("the captured request body is JSON");
    let mut names: Vec<String> = body["tools"]
        .as_array()
        .expect("an answering request advertises its tools")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

/// Runs one scenario and returns `(child output, captured request bodies)`.
fn scenario(label: &str, stated: &[&str]) -> (std::process::Output, Vec<String>) {
    let fixture = build_demo(label);
    let mut mock = spawn_mock();
    let output = run_ask(&fixture, mock.address(), stated);
    mock.join();
    let bodies = mock.bodies();
    let _ = fs::remove_dir_all(&fixture.root);
    (output, bodies)
}

#[test]
fn an_ask_without_the_flag_advertises_the_surface_it_always_did() {
    let (output, bodies) = scenario("ask-default", &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let names = advertised(&bodies);
    assert!(
        names.contains(&"request_clarification".to_string()),
        "the unstated surface keeps the clarification tool: {names:?}"
    );
    assert!(
        names.contains(&"schema_discovery".to_string())
            && names.contains(&"bounded_sql_query".to_string()),
        "the selected profile advertises the database tools: {names:?}"
    );
}

#[test]
fn a_stated_list_narrows_the_ask_surface_and_leaves_the_database_tools() {
    let (output, bodies) = scenario("ask-narrowed", &["read", "grep"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let names = advertised(&bodies);
    assert!(
        !names.contains(&"request_clarification".to_string()),
        "the list removes a tool the ask path composed: {names:?}"
    );
    assert!(
        names.contains(&"schema_discovery".to_string())
            && names.contains(&"bounded_sql_query".to_string()),
        "database tools follow the connection, not the list: {names:?}"
    );
    // The stated names survive exactly: the governed subset is the list, no
    // more and no less. A regression that emptied the advertisement, or that
    // dropped the filter's lower bound, fails here.
    let governed: Vec<&String> = names
        .iter()
        .filter(|name| !saya_config::is_database_tool(name))
        .collect();
    assert_eq!(
        governed,
        vec!["grep", "read"],
        "the list is the whole governed surface here: {names:?}"
    );
}

/// `--tools` is honoured by the surfaces that compose a tool list — the
/// interactive session, `ask`, `run`, and `saya start`. Every other
/// subcommand, including the `saya job` lifecycle route, refuses it the way
/// `--allow` / `--deny` / `--turn-file` are refused, rather than parsing it
/// and silently dropping a stated intent.
#[test]
fn a_subcommand_that_does_not_honour_the_flag_refuses_it() {
    let root = test_root("tools-refused-subcommand");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["job", "list", "--tools", "read"])
        .env("HOME", &root)
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "an unhonoured flag is refused, never ignored: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--tools") && stderr.contains("saya start"),
        "the refusal names the flag and the surfaces that read it: {stderr}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn naming_a_connection_tied_tool_is_refused_with_its_reason() {
    let (output, bodies) = scenario("ask-database-tool", &["schema_discovery"]);
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .contains("database tools follow the selected connection and the data-sharing setting"),
        "the refusal says why: {stderr}"
    );
    assert!(
        bodies.is_empty(),
        "a refused launch never reaches the provider: {bodies:?}"
    );
}

#[test]
fn naming_a_tool_that_does_not_exist_lists_the_valid_ones() {
    let (output, _bodies) = scenario("ask-unknown-tool", &["nope"]);
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"unknown tool "nope""#) && stderr.contains("valid names:"),
        "the refusal names the word and the alternatives: {stderr}"
    );
    assert!(
        stderr.contains("run_command") && !stderr.contains("scratch_sql"),
        "the valid names are the whole governed vocabulary: {stderr}"
    );
}

/// The scratch database is gone: its two tool names are not a governed
/// capability any more, so stating either is the ordinary unknown-tool
/// refusal — never a launch that silently advertises nothing.
#[test]
fn naming_a_removed_scratch_tool_is_refused_as_unknown() {
    for name in ["scratch_sql", "scratch_import"] {
        let (output, bodies) = scenario(&format!("ask-removed-{name}"), &[name]);
        assert_ne!(
            output.status.code(),
            Some(0),
            "{name} is not a governed tool: {output:?}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!(r#"unknown tool "{name}""#))
                && stderr.contains("valid names:"),
            "{name} is refused by name with the valid alternatives: {stderr}"
        );
        assert!(
            bodies.is_empty(),
            "a refused launch never reaches the provider: {bodies:?}"
        );
    }
}

/// Reads one HTTP request to completion: headers, then exactly the
/// `Content-Length` body bytes.
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

/// A mock OpenAI-compatible provider that always answers with plain text and
/// captures every request. The thread returns after a quiet period with no
/// new connection, so `join` never hangs.
struct MockProvider {
    address: String,
    bodies: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

fn spawn_mock() -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&bodies);
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let mut last_activity = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    last_activity = Instant::now();
                    stream.set_nonblocking(false).unwrap();
                    let request = drain_request(&mut stream);
                    captured.lock().unwrap().push(request);
                    write_response(&mut stream, &text_body(FALLBACK));
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

    fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }

    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
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
