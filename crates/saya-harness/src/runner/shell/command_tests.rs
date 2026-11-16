//! Windows-only: the platform shell's command string survives the spawn.
//!
//! These tests spawn `cmd.exe` through the same [`ShellCommandLine`] the
//! runner uses, in a fresh temp directory, and assert the files the shell
//! writes byte for byte. They guard the one thing a Unix `sh` cannot show:
//! `cmd.exe` does not understand Rust's MSVC argument quoting, so a command
//! passed as an ordinary argument loses its own quotes. They run under Wine
//! where the Windows target is built.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{ShellCommandLine, shell_argv, shell_program_path};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A fresh directory under the system temp root, removed on drop; the pid
/// and a counter keep parallel tests apart.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "saya-shell-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("the temp directory is created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The platform shell with `command`, spawned the way the runner spawns it:
/// the flag prefix as argv, the command through the shared helper.
fn std_shell(dir: &Path, command: &str) -> std::process::Output {
    let mut shell = Command::new(shell_program_path());
    shell.args(shell_argv()).current_dir(dir);
    shell.shell_command(command);
    shell.output().expect("the platform shell starts")
}

/// The same call over `tokio::process::Command`, proving the helper covers
/// both command types.
async fn tokio_shell(dir: &Path, command: &str) -> std::process::Output {
    let mut shell = tokio::process::Command::new(shell_program_path());
    shell.args(shell_argv()).current_dir(dir);
    shell.shell_command(command);
    shell.output().await.expect("the platform shell starts")
}

/// `echo "a b">out.txt`: cmd echoes the quotes and a CRLF. Windows also
/// keeps the space before the redirect; Wine 9 drops that one space, so the
/// contract asserted is the one both share. Rust's default argument quoting
/// would deliver `\"a b\"` instead — the backslashes are the regression.
fn assert_quoted_echo(content: &str) {
    assert!(
        content == "\"a b\"\r\n" || content == "\"a b\" \r\n",
        "cmd echoes the quotes and a CRLF: {content:?}"
    );
}

#[test]
fn a_quoted_echo_reaches_cmd_exactly() {
    let dir = TempDir::new("quoted-echo");
    let output = std_shell(dir.path(), "echo \"a b\">out.txt");
    assert!(output.status.success(), "the shell runs: {output:?}");
    assert_quoted_echo(
        &fs::read_to_string(dir.path().join("out.txt")).expect("the echo wrote out.txt"),
    );
}

/// The same call through `tokio::process::Command`: one helper, both command
/// types, no second quoting rule.
#[tokio::test]
async fn the_tokio_command_takes_the_same_raw_argument() {
    let dir = TempDir::new("tokio-echo");
    let output = tokio_shell(dir.path(), "echo \"a b\">out.txt").await;
    assert!(output.status.success(), "the shell runs: {output:?}");
    assert_quoted_echo(
        &fs::read_to_string(dir.path().join("out.txt")).expect("the echo wrote out.txt"),
    );
}

/// The shape the job check uses: an inner-quoted argument on the left of
/// `&&`, with the marker write only running if the first command succeeds.
/// The quotes must reach the program, not the shell's argument parser.
#[test]
fn an_and_chain_with_an_inner_quoted_argument_runs() {
    let dir = TempDir::new("and-chain");
    let output = std_shell(dir.path(), "echo \"a b\">out.txt && echo ok>check.txt");
    assert!(
        output.status.success(),
        "the shell runs the chain: {output:?}"
    );
    assert_quoted_echo(
        &fs::read_to_string(dir.path().join("out.txt")).expect("the first command wrote out.txt"),
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("check.txt")).expect("the chain wrote check.txt"),
        "ok\r\n",
        "the second command runs after the first succeeds"
    );
}
