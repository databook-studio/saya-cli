//! The trimmed mock-provider and fixture harness for the stopped-candidate-
//! vote boundary proof: the same shape as `tests/correctness/common.rs` (the
//! mock harness is duplicated per binary), plus a bounded child run so a
//! regression fails on a deadline instead of hanging the test.

use std::{
    fs,
    io::Read,
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
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
/// string so a scenario can assert on what saya sent.
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
/// script order; every request body is captured so the scenario can assert on
/// exactly which requests reached the provider. Serves exactly
/// `responses.len()` connections, then stops: an unexpected extra provider
/// call fails loudly (connection refused) rather than being absorbed.
pub(crate) struct MockProvider {
    address: String,
    bodies: Arc<Mutex<Vec<String>>>,
}

pub(crate) fn spawn_mock(responses: Vec<String>) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&bodies);
    thread::spawn(move || {
        for body in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = drain_request(&mut stream);
            captured.lock().unwrap().push(request);
            write_response(&mut stream, &body);
        }
    });
    MockProvider { address, bodies }
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
    /// the round the mock answered with the i-th scripted response.
    pub(crate) fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }
}

/// The isolated fixture a scenario runs: the real demo database plus the
/// config/state roots `saya ask` reads.
pub(crate) struct DemoFixture {
    pub(crate) root: PathBuf,
    pub(crate) connections: PathBuf,
    pub(crate) config_home: PathBuf,
    pub(crate) state_db: PathBuf,
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

/// What the bounded child run produced: its exit code (or `None` when it was
/// still running at the deadline), the captured streams, and whether the
/// deadline had to kill it.
pub(crate) struct BoundedRun {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) killed: bool,
}

/// Runs the real `saya ask` headlessly with a hard deadline: the child is
/// killed and the run marked if it has not exited in time, so a regression
/// hangs nothing.
pub(crate) fn run_ask_bounded(
    fixture: &DemoFixture,
    mock_address: &str,
    prompt: &str,
    deadline: Duration,
) -> BoundedRun {
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_saya"))
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the real saya binary spawns");
    // Drain both pipes on their own threads so a chatty child cannot wedge
    // itself on a full pipe while the wait loop polls.
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let out_thread = std::thread::spawn(move || {
        let mut buffer = String::new();
        let _ = stdout_pipe.read_to_string(&mut buffer);
        buffer
    });
    let err_thread = std::thread::spawn(move || {
        let mut buffer = String::new();
        let _ = stderr_pipe.read_to_string(&mut buffer);
        buffer
    });
    let started = Instant::now();
    let mut killed = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if started.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    killed = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("waiting on the ask child failed: {error}"),
        }
    };
    BoundedRun {
        code,
        stdout: out_thread.join().unwrap(),
        stderr: err_thread.join().unwrap(),
        killed,
    }
}