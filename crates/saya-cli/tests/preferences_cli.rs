//! `saya preferences` through the real binary (M04a).
//!
//! Every preference is explicit, per-worktree guidance the user confirmed by
//! running a command: a fresh worktree lists nothing, adds and edits and
//! forgets behave and report, secret-shaped text is refused and stored
//! nowhere, another worktree sees none of this worktree's preferences, a
//! run from a subdirectory of the same worktree sees the same ones (the
//! identity is the canonical root, not the launch cwd), and a run outside
//! any git worktree is refused with the worktree remedy. Every refusal
//! exits 2, the documented usage class. No path reads stdin.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn unique_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-cli-preferences-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A fake worktree for the binary: a directory holding an empty `.git`
/// directory — the walk-up in the workspace resolver stops exactly there.
fn fake_worktree(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

fn saya(root: &Path, cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .current_dir(cwd)
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(root: &Path, cwd: &Path, args: &[&str]) -> Output {
    saya(root, cwd, args).output().unwrap()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The first `pp-…` token in a listing — 35 chars, `pp-` plus 32 hex.
fn first_preference_id(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|token| token.len() == 35 && token.starts_with("pp-"))
        .map(str::to_owned)
}

fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

include!("preferences_cli/lifecycle.rs");

include!("preferences_cli/worktree_scope.rs");

include!("preferences_cli/cli_surface.rs");
