//! Tests 10–15: the host path. A call may ask for `sandbox: false`, and
//! where the caller allowed it the child runs on the host — the user's own
//! environment minus the removed names, the whole filesystem, no confinement.
//! Every test here builds a **host-only** shell, which needs no sandbox, so
//! none of them skips on an unproven host — except the mirror refusal, which
//! cannot exist without a proven spawn to refuse.
//!
//! 10. a host run may touch a path outside the workspace, and reports the
//!     path it ran in;
//! 11. the host child inherits this process's environment minus the removed
//!     names — a planted non-secret arrives, a removed secret does not;
//! 12. a declared secret value is scrubbed from the host capture;
//! 13. the host timeout kills the whole process group, leaving nothing alive;
//! 14. a workdir symlink out of the workspace is refused on a host run too;
//! 15. a call that asks for a path this shell does not have is refused
//!     before anything spawns.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use saya_types::WorkspacePath;

use super::{HOST_CHILD_ROOT, HOST_CHILD_TEST, TempRoot, alive, proven_shell};
use crate::runner::shell::{HostPolicy, WorkspaceShell};
use crate::runner::{RunnerError, ShellCall};

/// One host-only shell: the workspace root the *workdir* is confined to (a
/// host command itself may touch anything), the root's guard, and the tool.
pub(super) struct HostOnly {
    pub(super) root: PathBuf,
    pub(super) shell: WorkspaceShell,
    _guard: TempRoot,
}

/// Builds a host-only shell over a fresh scratch root: no sandbox is proven
/// or needed, so this constructor never skips.
pub(super) fn host_only_shell(tag: &str, host: HostPolicy) -> HostOnly {
    let guard = TempRoot::new(tag);
    let root = guard.canonical();
    let shell = WorkspaceShell::host_only(root.clone(), host)
        .expect("a host-only shell constructs over a real workspace root");
    HostOnly {
        root,
        shell,
        _guard: guard,
    }
}

/// A host run is not confined to the workspace: the command may write
/// anywhere the user can, and the outcome says which path it ran in.
#[tokio::test]
async fn a_host_run_may_touch_a_path_outside_the_workspace() {
    let host = host_only_shell("host-outside", HostPolicy::default());
    let outside = TempRoot::new("host-outside-target");
    let target = outside.0.join("landed");

    let outcome = host
        .shell
        .run(ShellCall::new(format!("touch '{}'", target.display())).with_sandbox(false))
        .await
        .expect("the host run must run");
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "the host write must succeed: {:?}",
        outcome.stderr.text
    );
    assert!(!outcome.sandboxed, "a host run reports the lane it ran in");
    assert!(
        target.is_file(),
        "the host write must land outside the workspace: {}",
        target.display()
    );
}

/// The host child inherits this process's environment — that is what a host
/// run is — minus the names the caller removed. The canaries are planted in
/// the *inner* process's environment by the outer one, so no `set_var` ever
/// touches the shared test binary's own environment.
#[tokio::test]
async fn a_host_run_removes_saya_secrets_and_keeps_the_rest() {
    if let Some(dir) = std::env::var_os(HOST_CHILD_ROOT) {
        // The inner run: both variables are in THIS process's environment,
        // so they are exactly what a host child would inherit.
        let dir = PathBuf::from(dir);
        let host = host_only_shell(
            "host-env-inner",
            HostPolicy {
                remove_env: vec!["SAYA_FAKE_SECRET".to_owned()],
                secret_values: vec!["s3cr3t-value".to_owned()],
            },
        );
        let outcome = host
            .shell
            .run(ShellCall::new("env").with_sandbox(false))
            .await
            .expect("the env dump must run");
        fs::write(dir.join("env.txt"), &outcome.stdout.text).expect("record the child's env");
        return;
    }

    let guard = TempRoot::new("host-env-outer");
    let status = Command::new(std::env::current_exe().expect("the test binary path resolves"))
        .arg("--exact")
        .arg(HOST_CHILD_TEST)
        .env(HOST_CHILD_ROOT, &guard.0)
        .env("KEEP_ME", "1")
        .env("SAYA_FAKE_SECRET", "s3cr3t-value")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("the self-re-exec must spawn");
    assert!(
        status.success(),
        "the inner host-env test must pass: {status}"
    );
    let text =
        fs::read_to_string(guard.0.join("env.txt")).expect("the inner run must record the env");
    assert!(
        text.contains("KEEP_ME=1"),
        "an ordinary variable of the host's environment must reach the child: {text}"
    );
    assert!(
        !text.contains("s3cr3t-value"),
        "a removed name's value must not reach the host child: {text}"
    );
}

/// A value the caller declared a secret is scrubbed from the capture, in
/// addition to the shared `redact()` pass — a bare value carries no marker
/// for the pattern pass to recognise.
#[tokio::test]
async fn host_captured_output_is_scrubbed_of_declared_secret_values() {
    let host = host_only_shell(
        "host-scrub",
        HostPolicy {
            remove_env: Vec::new(),
            secret_values: vec!["s3cr3t-value".to_owned()],
        },
    );
    let outcome = host
        .shell
        .run(ShellCall::new("echo s3cr3t-value").with_sandbox(false))
        .await
        .expect("the host run must run");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(
        !outcome.stdout.text.contains("s3cr3t-value"),
        "a declared secret value must not survive the capture: {:?}",
        outcome.stdout.text
    );
    assert!(
        outcome.stdout.text.contains("[redacted]"),
        "the redaction marker is what replaced it: {:?}",
        outcome.stdout.text
    );
}

/// The host path shares the sandboxed path's group kill: a background job is
/// exactly the process a single-child kill misses.
#[tokio::test]
async fn a_host_timeout_kills_the_process_group_and_leaves_nothing_alive() {
    let host = host_only_shell("host-timeout", HostPolicy::default());
    // A unique marker, so the liveness probe cannot match another test's
    // child or a stale process from an earlier run.
    let marker = format!("sleep 600.{:04}", std::process::id() % 10_000);
    let started = Instant::now();
    let outcome = host
        .shell
        .run(
            ShellCall::new(format!("{marker} & {marker}"))
                .with_sandbox(false)
                .with_timeout(Duration::from_secs(1)),
        )
        .await
        .expect("the host run must run");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "the timeout must fire well inside five seconds; took {elapsed:?}"
    );
    assert!(outcome.started, "the child did start");
    assert!(outcome.timed_out, "the timeout must be reported");
    assert!(!outcome.sandboxed, "the run reports the host lane");
    assert!(
        !alive(&marker),
        "no process from the killed group may survive the run"
    );
}

/// The workdir containment is the shell's own, not the sandbox's: a symlink
/// out of the workspace is refused on a host run too, before anything spawns.
#[tokio::test]
async fn a_workdir_symlink_out_of_the_workspace_is_refused_on_a_host_run() {
    let host = host_only_shell("host-workdir", HostPolicy::default());
    let outside = TempRoot::new("host-workdir-outside");
    std::os::unix::fs::symlink(&outside.0, host.root.join("escape"))
        .expect("the escape symlink must be creatable");

    let error = host
        .shell
        .run(
            ShellCall::new("pwd")
                .with_sandbox(false)
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

/// A call naming a path this shell does not have is refused before anything
/// spawns — in both directions, and the shell says which paths it holds.
#[tokio::test]
async fn a_call_asking_for_a_lane_this_shell_lacks_is_refused_before_it_spawns() {
    let host = host_only_shell("host-refusal", HostPolicy::default());
    assert!(
        !host.shell.has_sandbox(),
        "a host-only shell has no sandbox"
    );
    assert!(
        host.shell.allows_host(),
        "a host-only shell allows the host"
    );

    let error = host
        .shell
        .run(ShellCall::new("true"))
        .await
        .expect_err("a sandbox call must be refused where there is no sandbox");
    assert!(
        matches!(error, RunnerError::ShellSandboxUnavailable),
        "the refusal must be typed: {error:?}"
    );

    // The mirror needs a proven spawn to exist at all: on an unproven host
    // there is no sandbox-only shell to ask.
    let Some(proven) = proven_shell("host-refusal-proven") else {
        return;
    };
    assert!(proven.shell.has_sandbox(), "a proven shell has a sandbox");
    assert!(
        !proven.shell.allows_host(),
        "`new` composes the sandbox lane only"
    );
    let error = proven
        .shell
        .run(ShellCall::new("true").with_sandbox(false))
        .await
        .expect_err("a host call must be refused where the host is not allowed");
    assert!(
        matches!(error, RunnerError::ShellHostNotAllowed),
        "the refusal must be typed: {error:?}"
    );

    // `with_host` is what opens the host path on a sandboxed shell, and it
    // takes nothing away from the first.
    let both = proven.shell.with_host(HostPolicy::default());
    assert!(
        both.has_sandbox() && both.allows_host(),
        "with_host adds the host lane to a sandboxed shell"
    );
}
