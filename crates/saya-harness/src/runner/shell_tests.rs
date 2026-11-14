//! The workspace shell's battery. Nine guarantees, each a named test:
//!
//! 1. the parent's environment never reaches the shell child;
//! 2. a write outside the workspace is denied, one inside lands;
//! 3. egress to a live loopback listener is denied;
//! 4. a runaway stream is ring-capped and reported truncated;
//! 5. a timeout kills the whole process group, leaving nothing alive;
//! 6. cancellation does the same, and the run reports it started;
//! 7. a workdir symlink out of the workspace is refused before the spawn;
//! 8. captured output is redacted by the shared `redact()` pass;
//! 9. the call's bounds (command size, timeout) are typed refusals.
//!
//! The tests are grouped by what they exercise — `containment` (2, 3, 7),
//! `processes` (4, 5, 6, 9), `environment` (1, 8) — over the fixtures here.
//! `host` adds the host path (10–15): what a `sandbox: false` call may
//! reach, and the two refusals that keep a shell from running in a path its
//! caller did not compose. `background` adds background runs (16–24): an id
//! returned before the command ends, the output written since the last read,
//! the whole group killed on `shell_kill`, on the job's cancellation and when
//! the shell itself goes away, the cap that refuses rather than queues, and
//! the ring that bounds what a run no one has read may hold.
//!
//! Every test that spawns skips with a printed reason where the startup probe
//! did not prove the sandbox on this host — the startup-probe decision it
//! consumes. Test 1 plants its canary in a **child process's** environment
//! (the self-re-exec pattern `workspace::anchor_tests` uses): edition 2024
//! marks `set_var` unsafe because it is process-wide, and a shared lib test
//! has no business mutating the environment every other test in the binary
//! reads.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use super::sandbox::{RunSandbox, SandboxProvision};
use super::shell::{SHELL_PATH, WorkspaceShell};

#[path = "shell_tests/background.rs"]
mod background;
#[path = "shell_tests/containment.rs"]
mod containment;
#[path = "shell_tests/environment.rs"]
mod environment;
#[path = "shell_tests/host.rs"]
mod host;
#[path = "shell_tests/input.rs"]
mod input;
#[path = "shell_tests/processes.rs"]
mod processes;
#[path = "shell_tests/timeouts.rs"]
mod timeouts;

/// The measured reason `process-fork` is granted for: a shell forks for every
/// pipeline and every background job (the deny was measured without it —
/// `/bin/sh: fork: Operation not permitted`).
const FORK_REASON: &str = "the workspace shell runs pipelines and background jobs, which fork \
     - measured deny without the allow: /bin/sh: fork: Operation not permitted - spike 4.5";

/// The environment variable the outer canary test points its inner run at.
const CHILD_ROOT: &str = "SAYA_SHELL_CANARY_CHILD_ROOT";
/// The one test the inner run executes: the shell's env dump, recorded for
/// the outer run to inspect.
const CHILD_TEST: &str =
    "runner::shell_tests::environment::a_planted_parent_variable_never_reaches_the_shell";

/// The environment variable the outer host-env test points its inner run at.
const HOST_CHILD_ROOT: &str = "SAYA_SHELL_HOST_CHILD_ROOT";
/// The one test the inner host run executes: the host shell's env dump,
/// recorded for the outer run to inspect.
const HOST_CHILD_TEST: &str =
    "runner::shell_tests::host::a_host_run_removes_saya_secrets_and_keeps_the_rest";

/// Whether any process still matches `marker`, polled briefly: the group is
/// killed before the run returns, and a reaped child is gone at once — the
/// grace covers a zombie not yet collected by its new parent. Shared by the
/// two timeout tests (the sandboxed one and the host one): the guarantee is
/// the same one, so the probe is the same probe.
fn alive(marker: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let found = Command::new("pgrep")
            .arg("-f")
            .arg(marker)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|output| !output.stdout.is_empty());
        if !found || Instant::now() >= deadline {
            return found;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// A per-test scratch root: created, used, removed on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "saya-shell-{tag}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the shell test root must be creatable");
        Self(path)
    }

    fn canonical(&self) -> PathBuf {
        fs::canonicalize(&self.0).expect("the shell test root must canonicalise")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One proven shell: the workspace root it is confined to, the root's guard,
/// and the tool. `None` means this host's sandbox was not proven — the reason
/// has been printed.
struct Proven {
    root: PathBuf,
    shell: WorkspaceShell,
    _guard: TempRoot,
}

impl Proven {
    /// The same shell bound to `cancellation` — the job's token, as a
    /// composition root binds one. The background battery is the only caller:
    /// a background run dies with the token, so it needs one to fire.
    fn with_cancellation(mut self, cancellation: saya_agent::CancellationToken) -> Self {
        self.shell = self.shell.with_cancellation(cancellation);
        self
    }
}

/// Builds the shell the way a composition root would: the workspace as the
/// only filesystem root, no egress, `process-fork` granted with its measured
/// reason, and every `PATH` directory allowlisted for exec. A host whose
/// probe refuses yields `None`, and the caller skips.
fn proven_shell(tag: &str) -> Option<Proven> {
    proven_shell_denying(tag, None)
}

/// As [`proven_shell`], with `denied_name` joined to the workspace root and
/// added to the policy as a write denial. The macOS containment test is the
/// only caller with a denial: on Linux a denial is refused before a shell
/// exists at all.
fn proven_shell_denying(tag: &str, denied_name: Option<&str>) -> Option<Proven> {
    let guard = TempRoot::new(tag);
    let root = guard.canonical();
    let dirs: Vec<PathBuf> = SHELL_PATH.split(':').map(PathBuf::from).collect();
    let mut policy = RunSandbox::new([root.clone()], Vec::<(String, u16)>::new())
        .expect("a real workspace root constructs")
        .with_process_fork(FORK_REASON)
        .expect("the reason carries the safe profile text class");
    if let Some(name) = denied_name {
        policy = policy
            .with_write_denied(root.join(name))
            .expect("the denial is inside the workspace root");
    }
    let spawn = match policy
        .prepare_with_program_dirs(&dirs)
        .expect("preparation itself must run")
    {
        SandboxProvision::Proven { spawn, .. } => spawn,
        SandboxProvision::Refused(report) => {
            eprintln!(
                "skipping `{tag}`: the sandbox is not proven on this host:\n{}",
                report.render()
            );
            return None;
        }
    };
    let shell = WorkspaceShell::new(spawn).expect("the proven spawn is a shell confinement");
    Some(Proven {
        root,
        shell,
        _guard: guard,
    })
}
