//! A fresh machine for the first-run rows: the demo database, a private
//! config root that holds no `config.toml`, and a child whose environment is
//! cleared before the row states its one provider key.
//!
//! The scripted-provider pieces are shared with the other harnesses; this
//! module uses a subset and says so at the module declaration.

include!("../support/scripted_provider/provider_io.rs");
include!("../support/scripted_provider/provider_responses.rs");
include!("../support/scripted_provider/scripted_server.rs");
include!("../support/windows_system_env.rs");

use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// An isolated, pid-suffixed scratch root for one scenario.
pub(crate) fn test_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-first-run-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// The isolated fixture: the real demo database plus the config, state and
/// working roots the child reads. No `config.toml` is written anywhere.
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) connections: PathBuf,
    pub(crate) config_home: PathBuf,
    pub(crate) state_db: PathBuf,
    pub(crate) workspace: PathBuf,
}

pub(crate) fn fixture(label: &str) -> Fixture {
    let root = test_root(label);
    let demo_dir = root.join("demo");
    let config_home = root.join("user-config");
    let state_db = root.join("state.sqlite3");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_saya"))
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

/// The user config a row states: only the configured-provider row writes one.
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

/// Runs the real binary with a cleared environment plus exactly the variables
/// the row states, and a hard deadline.
fn run(fixture: &Fixture, args: &[&str], env: &[(&str, &str)], deadline: Duration) -> Ran {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .current_dir(&fixture.workspace)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Windows cannot open a socket without its system variables back; Unix
    // needs none of them.
    inherit_windows_system_env(&mut command);
    command
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &fixture.root)
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_STATE_DB", &fixture.state_db);
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("the real saya binary spawns");
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
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("waiting on the child failed: {error}"),
        }
    };
    Ran {
        code,
        stdout: out_thread.join().unwrap(),
        stderr: err_thread.join().unwrap(),
        killed,
    }
}

/// `saya ask` against the scripted endpoint, with no provider variable stated.
pub(crate) fn run_ask(
    fixture: &Fixture,
    address: &str,
    env: &[(&str, &str)],
    deadline: Duration,
) -> Ran {
    let connections = fixture.connections.to_str().unwrap();
    let mut vars = vec![("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))];
    vars.extend(env.iter().map(|(name, value)| (*name, value.to_string())));
    let refs: Vec<(&str, &str)> = vars
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    run(
        fixture,
        &[
            "--non-interactive",
            "--format",
            "text",
            "--connections",
            connections,
            "--profile",
            "demo",
            "--mode",
            "read-only",
            "ask",
            "hello",
        ],
        &refs,
        deadline,
    )
}

/// `saya config show`, the command that resolves the configuration without
/// talking to a provider or a database.
pub(crate) fn run_config_show(fixture: &Fixture, env: &[(&str, &str)], deadline: Duration) -> Ran {
    run(
        fixture,
        &["--format", "json", "config", "show"],
        env,
        deadline,
    )
}
