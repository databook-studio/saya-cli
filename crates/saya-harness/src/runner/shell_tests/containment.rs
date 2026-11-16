//! Tests 2, 3 and 7: what the shell child can reach. Every one of these is a
//! denial the workspace confinement is responsible for, and the first two are
//! paired with the control that proves the denial is the sandbox and not a
//! broken command.

use std::fs;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

use saya_types::WorkspacePath;

#[cfg(target_os = "linux")]
use super::FORK_REASON;
#[cfg(target_os = "macos")]
use super::proven_shell_denying;
use super::{TempRoot, proven_shell};
#[cfg(target_os = "linux")]
use crate::runner::sandbox::RunSandbox;
use crate::runner::{RunnerError, ShellCall};

/// A write above the workspace root fails and leaves nothing; a write to an
/// absolute path outside it fails too; a write inside lands. The last is the
/// control: without it a profile that denied everything would pass.
#[tokio::test]
async fn writes_outside_the_workspace_are_denied_and_inside_ones_land() {
    let Some(proven) = proven_shell("writes") else {
        return;
    };
    let outside_parent = proven
        .root
        .parent()
        .expect("the workspace root has a parent")
        .join("outside-x");
    let outside_tmp = std::env::temp_dir().join("outside-y");
    let _ = fs::remove_file(&outside_parent);
    let _ = fs::remove_file(&outside_tmp);

    let traversal = proven
        .shell
        .run(ShellCall::new("touch ../outside-x"))
        .await
        .expect("the shell must run");
    assert_ne!(
        traversal.exit_code,
        Some(0),
        "a write above the workspace root must fail: {:?}",
        traversal.stderr.text
    );
    assert!(
        !outside_parent.exists(),
        "the denied write must leave nothing behind: {}",
        outside_parent.display()
    );

    // The child's environment has no `TMPDIR` (the contract is four
    // variables), so this expands to the absolute path `/outside-y` — still
    // outside the root, and still denied.
    let temp = proven
        .shell
        .run(ShellCall::new("touch \"$TMPDIR/outside-y\""))
        .await
        .expect("the shell must run");
    assert_ne!(
        temp.exit_code,
        Some(0),
        "a write to an absolute path outside the workspace must fail: {:?}",
        temp.stderr.text
    );
    assert!(!outside_tmp.exists(), "the denied write must leave nothing");

    let inside = proven
        .shell
        .run(ShellCall::new("touch inside-z"))
        .await
        .expect("the shell must run");
    assert_eq!(
        inside.exit_code,
        Some(0),
        "a write inside the workspace must succeed: {:?}",
        inside.stderr.text
    );
    assert!(
        proven.root.join("inside-z").is_file(),
        "the allowed write must land in the workspace"
    );
}

/// Egress to a listener that is really listening is denied. Exit 127 is
/// excluded explicitly: that is `/bin/sh` reporting the program missing, and
/// a test that passed for that reason would prove nothing about the network.
#[tokio::test]
async fn egress_to_a_live_loopback_listener_is_denied() {
    let Some(proven) = proven_shell("egress") else {
        return;
    };
    // A genuinely accepting listener: the starved-listener lesson —
    // one that never calls `accept` turns a TCP timeout into a false denial.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("the listener must bind");
    let port = listener
        .local_addr()
        .expect("the listener's address")
        .port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
        }
    });

    let outcome = proven
        .shell
        .run(ShellCall::new(format!("nc -z 127.0.0.1 {port}")))
        .await
        .expect("the shell must run");
    assert_ne!(
        outcome.exit_code,
        Some(0),
        "egress must be denied: {:?}",
        outcome.stderr.text
    );
    assert_ne!(
        outcome.exit_code,
        Some(127),
        "`nc` must be exec-allowed, so the denial is the connect and not a missing \
         program: {:?}",
        outcome.stderr.text
    );
}

/// A write denial inside the workspace root is a real denial: the shell cannot
/// create the denied directory, cannot write into it once the parent process
/// has created it, and still writes everywhere else in the root. The last is
/// the control — without it a profile that denied the whole root would pass.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn writes_into_a_denied_subtree_are_denied() {
    let Some(proven) = proven_shell_denying("deny-write", Some(".git")) else {
        return;
    };
    let git = proven.root.join(".git");

    let made = proven
        .shell
        .run(ShellCall::new("mkdir .git"))
        .await
        .expect("the shell must run");
    assert_ne!(
        made.exit_code,
        Some(0),
        "creating the denied directory must fail: {:?}",
        made.stderr.text
    );
    assert!(
        !git.exists(),
        "the denied directory must not exist: {}",
        git.display()
    );

    // The parent process creates the directory the denial is about: the rule
    // denies writes *into* the subtree, not its absence.
    fs::create_dir(&git).expect("the test may create the denied directory itself");
    let touched = proven
        .shell
        .run(ShellCall::new("touch .git/x"))
        .await
        .expect("the shell must run");
    assert_ne!(
        touched.exit_code,
        Some(0),
        "a write inside the denied subtree must fail: {:?}",
        touched.stderr.text
    );
    assert!(
        !git.join("x").exists(),
        "the denied write must leave nothing behind"
    );

    let ok = proven
        .shell
        .run(ShellCall::new("touch ok"))
        .await
        .expect("the shell must run");
    assert_eq!(
        ok.exit_code,
        Some(0),
        "a write elsewhere in the root must land: {:?}",
        ok.stderr.text
    );
    assert!(
        proven.root.join("ok").is_file(),
        "the allowed write must land in the workspace"
    );
}

/// Linux is allow-only: Landlock cannot carve a denied path out of an allowed
/// tree, so preparation must refuse the whole policy rather than silently drop
/// the denial and hand out a wider sandbox than the caller asked for.
#[cfg(target_os = "linux")]
#[test]
fn a_write_denial_is_refused_at_prepare_time_on_linux() {
    use crate::runner::sandbox::{SandboxError, SandboxProvision};
    use crate::runner::shell::SHELL_PATH;

    let guard = TempRoot::new("deny-linux");
    let root = guard.canonical();
    let dirs: Vec<PathBuf> = SHELL_PATH.split(':').map(PathBuf::from).collect();
    let policy = RunSandbox::new([root.clone()], Vec::<(String, u16)>::new())
        .expect("a real workspace root constructs")
        .with_process_fork(FORK_REASON)
        .expect("the reason carries the safe profile text class")
        .with_write_denied(root.join(".git"))
        .expect("the denial is inside the workspace root");
    match policy.prepare_with_program_dirs(&dirs) {
        Err(SandboxError::WriteDenialNotExpressible { .. }) => {}
        Err(other) => panic!("the refusal must be typed: {other:?}"),
        // An unproven host never reaches the confinement builder; skip, as
        // every spawning test in this file does.
        Ok(SandboxProvision::Refused(report)) => eprintln!(
            "skipping `a_write_denial_is_refused_at_prepare_time_on_linux`: \
             the sandbox is not proven on this host:\n{}",
            report.render()
        ),
        Ok(SandboxProvision::Proven { .. }) => {
            panic!("a write denial must not yield a proven spawn on linux")
        }
    }
}

/// A workdir that is a symlink out of the workspace is refused before
/// anything spawns, and the escaped directory stays untouched.
#[tokio::test]
async fn a_workdir_symlink_out_of_the_workspace_is_refused() {
    let Some(proven) = proven_shell("workdir") else {
        return;
    };
    let outside = TempRoot::new("workdir-outside");
    std::os::unix::fs::symlink(&outside.0, proven.root.join("escape"))
        .expect("the escape symlink must be creatable");

    let error = proven
        .shell
        .run(
            ShellCall::new("pwd")
                .with_workdir(WorkspacePath::try_from("escape".to_owned()).expect("shaped path")),
        )
        .await
        .expect_err("a workdir outside the workspace must be refused");
    assert!(
        matches!(error, RunnerError::ShellCommandRefused { .. }),
        "the refusal must be typed: {error:?}"
    );
    assert!(
        fs::read_dir(&outside.0)
            .expect("the outside root must be readable")
            .next()
            .is_none(),
        "nothing may have run in the escaped directory"
    );
}
