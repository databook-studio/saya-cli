//! The real binary over a scripted provider: a private temporary root under the
//! workspace's `target/`, a Git repository to run `-p` in, and the child's
//! environment.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn git_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["git.exe", "git.cmd", "git.bat"]
    } else {
        &["git"]
    }
}

/// An explicit `SAYA_TEST_GIT` wins; every other environment finds the
/// platform's installed Git.
fn git_executable() -> PathBuf {
    if let Some(git) = std::env::var_os("SAYA_TEST_GIT") {
        return PathBuf::from(git);
    }
    let Some(path) = std::env::var_os("PATH") else {
        panic!("no Git on PATH and SAYA_TEST_GIT is unset");
    };
    std::env::split_paths(&path)
        .flat_map(|directory| git_names().iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
        .expect("installed Git")
}

/// A private root under the workspace's `target/`, removed by the caller.
pub(super) fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target");
    let root = workspace.join(format!(
        "tmp-oneshot-ndjson-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::canonicalize(root).unwrap()
}

pub(super) fn git(repo: &Path, args: &[&str]) {
    let output = Command::new(git_executable())
        .args([
            "-c",
            "user.name=ndjson",
            "-c",
            "user.email=ndjson@example.invalid",
            "-c",
            "core.hooksPath=",
            "-C",
        ])
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("Git starts");
    assert!(
        output.status.success(),
        "git {args:?} succeeds: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository for the one-shot job: one committed file, no operator.
pub(super) fn repository(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(root.join("config/saya")).unwrap();
    git(&repo, &["init", "--quiet"]);
    std::fs::write(repo.join("tracked.txt"), "baseline\n").unwrap();
    git(&repo, &["add", "--", "tracked.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "baseline"]);
    repo
}

/// Runs the real binary in `repo`, with every piece of state under `root` —
/// outside the repository, as the job state contract requires.
pub(super) fn saya(root: &Path, repo: &Path, provider: &str, args: &[&str]) -> Output {
    let git = git_executable();
    let path = std::env::join_paths(
        std::iter::once(git.parent().expect("Git has a parent").to_path_buf()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ),
    )
    .expect("PATH components are valid");
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .current_dir(repo)
        .env_clear()
        .env("PATH", path)
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("SAYA_CONFIG_HOME", root.join("config"))
        .env("SAYA_SESSION_DIR", root.join("sessions"))
        .env("SAYA_STATE_DB", root.join("state/saya/state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{provider}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    command
        .stdin(std::process::Stdio::null())
        .output()
        .expect("saya starts")
}

/// The run's stdout and stderr, ready to assert on.
pub(super) fn text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
