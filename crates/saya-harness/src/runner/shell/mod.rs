//! The workspace shell: one operation — `WorkspaceShell::run` — that
//! runs `/bin/sh -c <command>` inside the run's workspace, under the same
//! proven sandbox the typed-argv runner uses.
//!
//! The shell is deliberately *not* a second spawn path. It resolves to a
//! [`ValidatedCall`] and hands it to the one `execute` in `spawn`, so the
//! confinement, the empty-then-declared environment, the output ring, the
//! `redact()` pass, and the process-group kill are the ones already measured
//! — there is no `tokio::process::Command` here and no inherited environment.
//!
//! What the shell adds to that shared path is exactly three decisions, and
//! each one is resolved **once**, before anything spawns:
//!
//! - **The command.** 1..=65,536 bytes and no NUL; anything else is a typed
//!   [`RunnerError::ShellCommandRefused`]. A shell is admitted here because
//!   the workspace confinement, not an allowlist, is the control: the child
//!   can reach nothing the workspace does not already contain.
//! - **The working directory.** A `WorkspacePath` is joined to the canonical
//!   workspace root and canonicalised; the *resolved* `PathBuf` must still
//!   start with that root, so a symlink that leaves the workspace is refused
//!   before the spawn. The resolved path is the child's `current_dir`; the
//!   string is never re-joined afterwards.
//! - **The environment.** Empty but for `PATH`, `HOME` (the workspace root,
//!   or — through [`WorkspaceShell::with_scratch`] — a helper's own private
//!   scratch home, with `TMPDIR` beside it), `LANG` and `TERM`. No credential
//!   is reachable from this path: the credential list is empty by
//!   construction, so the child cannot echo a value the parent holds.
//!
//! # The host path
//!
//! A call carries a `sandbox` flag. `true` is the sandboxed path above, unchanged, and
//! is the default. `false` asks for a **host run**: the same `/bin/sh -c
//! <command>`, the same resolved workdir, the same ring and redaction gate,
//! the same process-group kill — but the child is a plain `Command::new`, it
//! inherits this process's environment minus [`HostPolicy::remove_env`], and
//! nothing confines it. Whether that path exists is the *caller's* decision,
//! resolved once at construction and never by the model: [`new`] composes
//! the sandbox path alone, [`host_only`] the host path alone where no
//! sandbox is proven, and [`with_host`] both. A call naming a path the shell
//! does not hold is refused before anything spawns — the tool never picks
//! the other path on the caller's behalf.
//!
//! Both paths reach the child through one `spawn::execute`, so the
//! confinement differs and nothing else does.
//!
//! # Background runs
//!
//! `start_background` is the same call with one thing changed: the shell does
//! not wait for the child. It takes the same path decision, the same
//! validation and the same confinement, and the run it makes is bounded,
//! reachable and killed with its surface — the shell itself, or the longer
//! lived owner whose registry the shell shares — see [`background`] for the
//! three bounds. A shell that does not outlive a call must not offer it, so
//! whether a shell has this path is its caller's decision, exactly like the
//! host path.
//!
//! The caller supplies the proven [`RunnerSpawn`] — this type is the tool
//! built around it — and is responsible for preparing that policy for a
//! shell: the `PATH` directories allowlisted for exec, and `process-fork`
//! granted with a measured reason, because a shell forks for every pipeline
//! and every background job.
//!
//! [`ValidatedCall`]: ValidatedCall

use std::{ffi::OsString, path::PathBuf, time::Duration};

use saya_agent::CancellationToken;

use super::OUTPUT_CAP_BYTES;
use super::RunnerError;
use super::output::{ProgramOutcome, StreamCapture};
use super::sandbox::RunnerSpawn;
use super::spawn;

mod background;
mod call;
mod detach;
mod host;
mod lanes;
mod toolchain;
mod validate;

pub use background::{BackgroundRuns, MAX_BACKGROUND_RUNS, ReadReport, RunStatus};
pub use call::{ShellCall, ShellOutcome};
pub use host::HostPolicy;
pub use toolchain::{
    MAX_TOOLCHAIN_DIRS, path_with_toolchain, toolchain_dirs, toolchain_dirs_for_process,
};

pub(super) use validate::ValidatedCall;

/// The one program this tool execs. A path, not a name: the shell is the
/// allowlisted program, and the sandbox profile is what confines what it can
/// then reach.
pub const SHELL_PROGRAM: &str = "/bin/sh";

/// The child's whole `PATH`. `/bin` holds `sh` and the shell's own utilities;
/// `/usr/bin` holds the programs the tests and a real command need — no single
/// directory holds both, which is why the spawn is prepared with all of them.
pub const SHELL_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// The configured default when a call names no timeout: two minutes, the
/// foreground bound a model-issued search or build gets before it is moved to
/// the background.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// The ceiling a call may not exceed: 600 s. A call may narrow the default or
/// widen it up to the ceiling, and zero is a typo rather than an instant
/// pause.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(600);

/// The command's byte ceiling: 64 KiB.
pub const MAX_COMMAND_BYTES: usize = 64 * 1024;

/// The stdin payload's byte ceiling. The payload is written **before** the
/// wait and the write is not raced, so the bound is also the invariant that
/// keeps it from blocking: a fresh pipe holds at least 16 KiB on every
/// platform this shell runs on (macOS), and a write that fits the pipe's
/// capacity completes whether or not the child ever reads. Eight KiB is half
/// of that floor, so a child that ignores its stdin cannot hold the call.
pub const MAX_STDIN_BYTES: usize = 8192;

const _: () = assert!(MAX_STDIN_BYTES < 16 * 1024);

/// The admitted workspace shell: the paths its caller composed, the
/// workspace root its children's workdir is confined to, and the fixed
/// environment a sandboxed child gets.
pub struct WorkspaceShell {
    /// The proven sandbox, when this shell has one: the only path a
    /// `sandbox: true` call can take.
    spawn: Option<RunnerSpawn>,
    /// The host policy, when a `sandbox: false` call is allowed.
    host: Option<HostPolicy>,
    root: PathBuf,
    /// The host toolchain directories on the sandboxed child's `PATH`. Kept
    /// here so a scratch-home rebuild cannot lose them.
    toolchain: Vec<PathBuf>,
    env: Vec<(&'static str, OsString)>,
    cancellation: CancellationToken,
    /// The runs this shell started and has not finished reading: bounded by
    /// [`MAX_BACKGROUND_RUNS`], killed by `cancellation`, and gone with the
    /// shell — or with the registry's last owner, where several shells share
    /// one (a session composes a shell per turn).
    background: std::sync::Arc<BackgroundRuns>,
    /// The token that ends the background runs. A shell composed per call uses
    /// its own `cancellation`; a session's per-turn shells use the session's
    /// token, so a run outlives the turn that started it.
    background_cancellation: Option<CancellationToken>,
    /// Whether a foreground call that hits its timeout is moved into the
    /// background registry instead of killed. The caller's decision, taken
    /// with the background family: a shell that does not advertise
    /// `shell_output` must not leave a run the model cannot read.
    detach_on_timeout: bool,
    /// The per-stream capture cap. [`OUTPUT_CAP_BYTES`] unless the caller is
    /// saving a large result's full output under the job's state directory, in
    /// which case the file, not the model, is the consumer that needs the
    /// larger window.
    capture_cap: usize,
}

impl WorkspaceShell {
    /// Builds the shell over a proven spawn, with the sandbox path and no
    /// host path. The spawn must be confined to a workspace with no egress —
    /// the workspace root is its first filesystem root, and a policy that
    /// allows network is refused rather than narrowed. A call that asks for
    /// the host is a [`RunnerError::ShellHostNotAllowed`] until
    /// [`with_host`](Self::with_host) opens that path.
    pub fn new(spawn: RunnerSpawn) -> Result<Self, RunnerError> {
        if !spawn.net_allow().is_empty() {
            return Err(RunnerError::ShellNotConfined {
                detail: "the proven sandbox allows network egress; the workspace shell runs \
                         with none",
            });
        }
        let root = spawn
            .fs_roots()
            .first()
            .ok_or(RunnerError::ShellNotConfined {
                detail: "the proven sandbox has no filesystem root to confine the shell to",
            })?
            .clone();
        let toolchain = toolchain::toolchain_dirs_for_process();
        Ok(Self {
            spawn: Some(spawn),
            host: None,
            env: validate::shell_env(&root, &toolchain),
            root,
            toolchain,
            cancellation: CancellationToken::new(),
            background: std::sync::Arc::new(BackgroundRuns::default()),
            background_cancellation: None,
            detach_on_timeout: false,
            capture_cap: OUTPUT_CAP_BYTES,
        })
    }

    /// Widens the per-stream capture window to `cap`. The model never reads
    /// the whole window: the caller's inline bound is smaller, and the extra
    /// retained bytes exist only so the caller can save them for the operator.
    /// The cap is explicit and still bounds memory.
    pub fn with_capture_cap(mut self, cap: usize) -> Self {
        self.capture_cap = cap;
        self
    }

    /// Attaches the run's cancellation token: a cancelled run kills the
    /// child's whole process group and reports the cancellation.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Shares a background registry with the other shells of one surface, and
    /// ends its runs on `cancellation` rather than on this shell's token. A
    /// session composes a shell per turn but owns its runs for the session:
    /// each turn's shell starts, reads and kills through the same registry,
    /// and the session's token — or the registry's last owner going away —
    /// ends every group. A shell that does not call this keeps its own
    /// registry, ended with the shell.
    pub fn with_shared_background(
        mut self,
        runs: std::sync::Arc<BackgroundRuns>,
        cancellation: CancellationToken,
    ) -> Self {
        self.background = runs;
        self.background_cancellation = Some(cancellation);
        self
    }

    /// Moves a foreground call that hits its timeout into the background
    /// registry instead of killing it. The caller's decision, taken with the
    /// background family: a shell that does not advertise `shell_output`
    /// keeps the kill, so it never leaves a run the model cannot read.
    pub fn with_timeout_detach(mut self, detach: bool) -> Self {
        self.detach_on_timeout = detach;
        self
    }

    /// The canonical workspace root the shell is confined to.
    pub fn workspace_root(&self) -> &PathBuf {
        &self.root
    }

    /// Points the **sandboxed** child's `HOME` and `TMPDIR` at a caller-owned
    /// scratch tree instead of the workspace root: a helper's own private
    /// `.saya/helper-<agent-id>/{home,tmp}`, so two helpers writing one
    /// checkout never share a home or a temp directory. `PATH`, `LANG` and
    /// `TERM` are unchanged, and the host path is untouched — it inherits this
    /// process's environment, which is what a host run is.
    pub fn with_scratch(mut self, home: PathBuf, tmpdir: PathBuf) -> Self {
        self.env = validate::shell_env_with_scratch(&home, Some(&tmpdir), &self.toolchain);
        self
    }

    /// Runs one call to completion and returns the child's report. Every
    /// refusal the call can earn happens here, before the spawn. A call that
    /// asked to run in the background is refused: it is started by
    /// [`start_background`](Self::start_background), and a flag this path
    /// ignored would be a flag the caller believed.
    ///
    /// A shell that [detaches on timeout](Self::with_timeout_detach) hands
    /// the still-running child to its background registry when the timeout
    /// fires; the run keeps the same lane, environment and bounds, and the
    /// caller gets its id. With no registry slot free, the call keeps the
    /// kill and reports `timed_out`.
    pub async fn run(&self, call: ShellCall) -> Result<ShellOutcome, RunnerError> {
        if call.background {
            return Err(refused(
                "a background call is started, not run: the shell does not wait for it",
            ));
        }
        let sandboxed = self.resolve_lane(&call)?;
        let (validated, cwd) = self.validated_call(&call)?;
        if !self.detach_on_timeout {
            let outcome = spawn::run_shell(
                self.exec_target(sandboxed),
                validated,
                &cwd,
                &self.cancellation,
                self.capture_cap,
            )
            .await?;
            return Ok(finished(outcome, sandboxed));
        }
        let timeout = validated.timeout;
        let unread = spawn::unread_rings(self.capture_cap);
        match detach::run(
            self.exec_target(sandboxed),
            validated,
            &cwd,
            &self.cancellation,
            unread,
            self.capture_cap,
        )
        .await?
        {
            detach::ShellRun::Finished(outcome) => Ok(finished(outcome, sandboxed)),
            detach::ShellRun::Detached(detached) => {
                match self.background.adopt(detached, self.background_watch()) {
                    Ok(id) => Ok(moved(id, timeout, sandboxed)),
                    Err(detached) => Ok(finished(detached.kill_timed_out()?, sandboxed)),
                }
            }
        }
    }
}

/// The outcome of a run that ended, killed, or was cancelled.
fn finished(outcome: ProgramOutcome, sandboxed: bool) -> ShellOutcome {
    ShellOutcome {
        started: true,
        exit_code: outcome.exit_code,
        timed_out: outcome.killed_by_timeout,
        cancelled: outcome.cancelled,
        sandboxed,
        truncated: outcome.stdout.truncated || outcome.stderr.truncated,
        stdout: outcome.stdout,
        stderr: outcome.stderr,
        moved_to_background: None,
        moved_after: None,
    }
}

/// The outcome of a run moved into the background: the id it is read by, and
/// the timeout that moved it. The output written before the move stays in the
/// run's unread ring, so the first `shell_output` returns it.
fn moved(id: u64, after: Duration, sandboxed: bool) -> ShellOutcome {
    ShellOutcome {
        started: true,
        exit_code: None,
        timed_out: true,
        cancelled: false,
        sandboxed,
        truncated: false,
        stdout: StreamCapture::empty(),
        stderr: StreamCapture::empty(),
        moved_to_background: Some(id),
        moved_after: Some(after),
    }
}

pub(super) fn refused(detail: impl Into<String>) -> RunnerError {
    RunnerError::ShellCommandRefused {
        detail: detail.into(),
    }
}

/// The output cap the capture honours, re-exported here so a caller of the
/// shell does not have to reach through the runner module for it. It is the
/// whole budget for one stream, split between the head and tail windows the
/// capture keeps; a truncated stream renders inside it, marker included.
pub const SHELL_OUTPUT_CAP_BYTES: usize = OUTPUT_CAP_BYTES;
