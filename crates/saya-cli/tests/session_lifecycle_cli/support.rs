use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, SystemTime},
};

pub(super) struct TestEnv {
    pub(super) root: PathBuf,
    pub(super) sessions: PathBuf,
}

pub(super) fn test_env(label: &str) -> TestEnv {
    let root = std::env::temp_dir().join(format!(
        "saya-cli-session-lifecycle-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    TestEnv { root, sessions }
}

pub(super) fn saya(env: &TestEnv, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

pub(super) fn run(env: &TestEnv, args: &[&str]) -> Output {
    saya(env, args).output().unwrap()
}

pub(super) fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(super) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub(super) fn entries(path: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(path)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Pins the file's modification time to `ago` before now, so listing order
/// is deterministic regardless of write timing.
pub(super) fn age(path: &Path, ago: Duration) {
    let mtime = SystemTime::now().checked_sub(ago).unwrap();
    let file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.set_modified(mtime).unwrap();
}
