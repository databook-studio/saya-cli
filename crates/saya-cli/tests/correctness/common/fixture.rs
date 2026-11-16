use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

/// The longest a scenario waits for one `saya` child. Building the demo and
/// answering one ask round are seconds of work; a child that outlives this is
/// wedged, and waiting on it forever would turn one wedged child into a
/// six-hour CI job. The deadline bounds the wait; it never skips an assertion.
const CHILD_TIMEOUT: Duration = Duration::from_secs(300);

/// Drains one of the child's pipes on its own thread: a full pipe buffer
/// would wedge the child as surely as a deadlock would.
fn drain<R: Read + Send + 'static>(mut pipe: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = pipe.read_to_end(&mut buffer);
        buffer
    })
}

/// Runs a child to completion within [`CHILD_TIMEOUT`], panicking — after
/// killing it — if it does not finish.
pub(crate) fn output_within(command: &mut Command, label: &str) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{label} starts: {error}"));
    let mut stdout = Some(drain(
        child.stdout.take().expect("the child's stdout is piped"),
    ));
    let mut stderr = Some(drain(
        child.stderr.take().expect("the child's stderr is piped"),
    ));
    let deadline = Instant::now() + CHILD_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("the child's status is observable") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let stderr = stderr
                    .take()
                    .expect("the stderr drain is still owned")
                    .join()
                    .expect("the stderr drain finishes");
                panic!(
                    "{label} did not finish within {}s; stderr: {}",
                    CHILD_TIMEOUT.as_secs(),
                    String::from_utf8_lossy(&stderr),
                );
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    };
    Output {
        status,
        stdout: stdout
            .take()
            .expect("the stdout drain is still owned")
            .join()
            .expect("the stdout drain finishes"),
        stderr: stderr
            .take()
            .expect("the stderr drain is still owned")
            .join()
            .expect("the stderr drain finishes"),
    }
}

/// An isolated, pid-suffixed scratch root for one scenario.
pub(crate) fn test_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-correctness-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
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
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(["demo", "--non-interactive", "--format", "json"])
        .env("SAYA_DEMO_DIR", &demo_dir)
        .env("SAYA_CONFIG_HOME", &config_home)
        .env("SAYA_STATE_DB", &state_db)
        .env("HOME", &root);
    let output = output_within(&mut command, "saya demo");
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
    output_within(&mut command, "saya ask")
}

/// Direct `saya query`: the independent truth for a scenario.
pub(crate) fn run_query(fixture: &DemoFixture, sql: &str) -> Vec<Vec<serde_json::Value>> {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_saya"));
    command
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
        .env("HOME", &fixture.root);
    let output = output_within(&mut command, "saya query");
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
