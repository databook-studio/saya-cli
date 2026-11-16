use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// The slash commands the 0.4.2 sections of docs/commands.md document.
pub(crate) const Q04_COMMANDS: &[&str] = &[
    "/notes",
    "/recovery",
    "/rotate",
    "/reload",
    "/setup",
    "/preferences",
    "/sessions",
    "/context",
];

pub(crate) struct Env {
    pub(crate) root: PathBuf,
    project: PathBuf,
    sessions: PathBuf,
}

/// An isolated scratch environment, shaped like the instruction-examples
/// suite: config home `root`, a fake git worktree at `root/project`, and
/// sessions/state outside the worktree.
pub(crate) fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!(
        "saya-docs-claims-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("saya")).unwrap();
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    Env {
        root,
        project,
        sessions,
    }
}

/// The real binary in the isolated roots; the provider env points at a dead
/// address that slash-command turns never contact.
pub(crate) fn saya(env: &Env) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .current_dir(&env.project)
        .env("SAYA_CONFIG_HOME", &env.root)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", "http://127.0.0.1:9/v1")
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// One slash-command turn via `--turn-file`.
pub(crate) fn slash_turn(env: &Env, name: &str, line: &str) -> std::process::Output {
    let turn = env.project.join(name);
    std::fs::write(&turn, format!("{line}\n")).unwrap();
    saya(env)
        .args([
            "--workspace",
            ".",
            "--approval-mode",
            "read-only",
            "--format",
            "ndjson",
            "--turn-file",
            turn.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

pub(crate) fn exit_zero(output: &std::process::Output, what: &str) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{what} must exit zero; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
