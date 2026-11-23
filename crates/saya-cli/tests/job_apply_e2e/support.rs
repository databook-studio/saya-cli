//! Process and repository fixtures for the managed-job e2e: the platform's
//! Git, the child's `PATH`, and a private temporary root.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn git_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["git.exe", "git.cmd", "git.bat"]
    } else {
        &["git"]
    }
}

/// An explicit `SAYA_TEST_GIT` wins: a Wine run has no Windows Git on the
/// container's `PATH`, so the harness points this at a real `git.exe`. Every
/// other environment finds the platform's installed Git.
pub(super) fn git_executable() -> PathBuf {
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

/// The child's `PATH`, with Git's directory first so the binary under test
/// finds the same Git this test drives. A Wine process may inherit no `PATH`
/// at all; the product supplies its own Windows base search path in that
/// case, so the Git directory alone is enough here.
pub(super) fn child_path(git: &Path) -> std::ffi::OsString {
    let directory = git.parent().expect("Git has a parent directory");
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(
        std::iter::once(directory.to_path_buf()).chain(std::env::split_paths(&inherited)),
    )
    .expect("PATH components are valid")
}

// The Windows system variables both cleared-environment harnesses keep.
include!("../support/windows_system_env.rs");

pub(super) fn git(repo: &Path, args: &[&str]) {
    let output = Command::new(git_executable())
        .args([
            "-c",
            "user.name=job-e2e",
            "-c",
            "user.email=job-e2e@example.invalid",
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

pub(super) fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-job-e2e-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::canonicalize(root).unwrap()
}
