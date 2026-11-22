//! The scripted provider, the demo fixture and the bounded `saya ask` child the
//! one-turn ask rows run against.
//!
//! The mock harness is duplicated per test binary (as `candidate_vote_stop` and
//! `correctness` do): one SSE reply per connection, in script order, each held
//! for the step's own delay before it is written — the held reply is what a
//! turn's own elapsed budget runs out against.

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

// The scripted provider and the wire bodies the rows script: one file, the way
// the other duplicated harnesses compose theirs.
include!("provider.rs");

/// An isolated, pid-suffixed scratch root for one scenario.
pub(crate) fn test_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-ask-session-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// The isolated fixture a row runs against: the real demo database plus the
/// config, state and working roots `saya ask` reads.
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) connections: PathBuf,
    pub(crate) config_home: PathBuf,
    pub(crate) state_db: PathBuf,
    /// The folder the child is run in: the session's folder.
    pub(crate) workspace: PathBuf,
}

pub(crate) fn fixture(label: &str) -> Fixture {
    let root = test_root(label);
    let demo_dir = root.join("demo");
    let config_home = root.join("user-config");
    let state_db = root.join("state.sqlite3");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
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
        String::from_utf8_lossy(&output.stderr)
    );
    let connections = demo_dir.join("connections.toml");
    assert!(connections.exists());
    Fixture {
        root,
        connections,
        config_home,
        state_db,
        workspace,
    }
}

/// The user config the child reads: one layer, stated by the row itself.
pub(crate) fn write_config(fixture: &Fixture, body: &str) {
    let home = fixture.config_home.join("saya");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.toml"), body).unwrap();
}

/// What the bounded child run produced.
pub(crate) struct Ran {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) killed: bool,
}

/// Runs the real `saya ask` in the fixture's workspace with a hard deadline.
pub(crate) fn run_ask(fixture: &Fixture, address: &str, prompt: &str, deadline: Duration) -> Ran {
    run_ask_after(fixture, address, prompt, None, deadline)
}

/// The same, with the operator's Ctrl-C delivered `after` the child started:
/// the signal an interactive terminal sends mid-turn.
#[cfg(unix)]
pub(crate) fn run_ask_interrupted(
    fixture: &Fixture,
    address: &str,
    prompt: &str,
    after: Duration,
    deadline: Duration,
) -> Ran {
    run_ask_after(fixture, address, prompt, Some(after), deadline)
}

fn run_ask_after(
    fixture: &Fixture,
    address: &str,
    prompt: &str,
    interrupt: Option<Duration>,
    deadline: Duration,
) -> Ran {
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "text",
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
        .current_dir(&fixture.workspace)
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &fixture.state_db)
        .env("SAYA_ALLOW_DATA_SHARING", "true")
        .env("HOME", &fixture.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the real saya binary spawns");
    let mut out_pipe = child.stdout.take().unwrap();
    let mut err_pipe = child.stderr.take().unwrap();
    let out_thread = thread::spawn(move || {
        let mut buffer = String::new();
        let _ = out_pipe.read_to_string(&mut buffer);
        buffer
    });
    let err_thread = thread::spawn(move || {
        let mut buffer = String::new();
        let _ = err_pipe.read_to_string(&mut buffer);
        buffer
    });
    let started = Instant::now();
    let mut killed = false;
    let mut signalled = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if let Some(after) = interrupt
                    && !signalled
                    && started.elapsed() >= after
                {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(child.id() as libc::pid_t, libc::SIGINT)
                    };
                    signalled = true;
                }
                if started.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    killed = true;
                    break None;
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("waiting on the ask child failed: {error}"),
        }
    };
    Ran {
        code,
        stdout: out_thread.join().unwrap(),
        stderr: err_thread.join().unwrap(),
        killed,
    }
}

/// The job directories the child left under its own coding state root: the one
/// factor a run of the chat composition states and the old ask turn body never
/// did.
pub(crate) fn jobs(fixture: &Fixture) -> Vec<String> {
    let coding = fixture
        .state_db
        .parent()
        .expect("the state db has a directory")
        .join("coding");
    let Ok(entries) = fs::read_dir(&coding) else {
        return Vec::new();
    };
    let mut jobs: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        // A job's directory is the one holding its own request.
        .filter(|entry| entry.path().join("request.json").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    jobs.sort();
    jobs
}
