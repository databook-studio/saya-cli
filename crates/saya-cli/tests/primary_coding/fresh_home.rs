//! The real binary on a fresh machine: no saya state directory exists yet, so
//! the coding commands must create what they need (privately) instead of
//! failing with a generic owner error. Acceptance for the setup command.
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-C",
        ])
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

struct Fresh {
    root: PathBuf,
    repo: PathBuf,
}

fn fresh(label: &str) -> Fresh {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "saya-fresh-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "baseline"]);
    Fresh { root, repo }
}

fn saya(fresh: &Fresh) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .current_dir(&fresh.repo)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", fresh.root.join("home"))
        .env("SAYA_CONFIG_HOME", fresh.root.join("config"))
        // Neither this directory nor its parent exists yet.
        .env(
            "SAYA_STATE_DB",
            fresh.root.join("fresh-state/saya/state.sqlite3"),
        )
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", "http://127.0.0.1:9/v1");
    command
}

#[test]
fn coding_setup_on_a_fresh_machine_prints_the_checklist_not_a_generic_error() {
    let fresh = fresh("setup");
    let output = saya(&fresh)
        .args(["--format", "json", "coding", "setup"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("native coding operation failed")
            && !stderr.contains("native coding operation failed"),
        "stdout={stdout} stderr={stderr}"
    );
    // The result is JSON carried as a string in the envelope: match its keys.
    assert!(
        stdout.contains("steps"),
        "the checklist is printed: {stdout} {stderr}"
    );
    assert!(
        stdout.contains("coding_enabled"),
        "coding is disabled by default and setup says what enables it: {stdout}"
    );
    let state = fresh.root.join("fresh-state/saya/coding");
    assert!(state.is_dir(), "the coding state directory was created");
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&state).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "created private");
    }
    let _ = std::fs::remove_dir_all(&fresh.root);
}
