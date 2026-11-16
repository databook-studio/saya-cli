//! What a shell call *is*, and what one run *reported*: the two shapes the
//! shell's one operation takes in and gives back. Both are data — the request
//! a caller fills in, the honest account a caller reads — and neither
//! carries behaviour, so they live here rather than beside the confinement
//! logic that consumes them.

use std::time::Duration;

use saya_types::WorkspacePath;

use crate::runner::StreamCapture;

/// One workspace shell call.
#[derive(Debug, Clone)]
pub struct ShellCall {
    /// The command text handed to `/bin/sh -c`.
    pub command: String,
    /// The child's working directory, relative to the workspace root.
    /// `None` means the root itself.
    pub workdir: Option<WorkspacePath>,
    /// The call's timeout. `None` means [`DEFAULT_TIMEOUT`].
    ///
    /// [`DEFAULT_TIMEOUT`]: super::DEFAULT_TIMEOUT
    pub timeout: Option<Duration>,
    /// Whether the call runs in the proven sandbox (`true`, the default) or
    /// on the host (`false`). Asking for a path the shell does not hold is a
    /// typed refusal, never a silent fallback to the other path.
    pub sandbox: bool,
    /// Whether the call is started and left running (`true`) instead of waited
    /// for (`false`, the default). The path, the confinement, and the bounds
    /// are the same either way; only the wait differs. A background call is
    /// refused by [`WorkspaceShell::run`](super::WorkspaceShell::run) and
    /// started by [`WorkspaceShell::start_background`](super::WorkspaceShell::start_background),
    /// so the flag is never quietly ignored.
    pub background: bool,
    /// Bytes written to the child's stdin and then closed. `None` leaves the
    /// child's stdin at EOF, as an inherited null device would. Bounded by
    /// [`MAX_STDIN_BYTES`](super::MAX_STDIN_BYTES).
    pub input: Option<Vec<u8>>,
}

impl ShellCall {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            workdir: None,
            timeout: None,
            sandbox: true,
            background: false,
            input: None,
        }
    }

    pub fn with_workdir(mut self, workdir: WorkspacePath) -> Self {
        self.workdir = Some(workdir);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Asks for the sandboxed path (`true`) or the host path (`false`).
    pub fn with_sandbox(mut self, sandbox: bool) -> Self {
        self.sandbox = sandbox;
        self
    }

    /// Asks for a background start (`true`) or a call the shell waits for
    /// (`false`, the default).
    pub fn with_background(mut self, background: bool) -> Self {
        self.background = background;
        self
    }

    /// Carries `input` to the child's stdin and closes the pipe, so the child
    /// reads exactly these bytes and then sees EOF. The bound is checked with
    /// every other bound, before anything spawns.
    pub fn with_input(mut self, input: impl Into<Vec<u8>>) -> Self {
        self.input = Some(input.into());
        self
    }
}

/// What one shell run reported. `started` is the honest account the caller
/// needs: an `Ok` outcome is only ever produced after the child was created,
/// so a run that started is never reported as "no effect" — a pre-spawn
/// refusal is an [`Err`], never a quiet empty outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutcome {
    /// Always true in an `Ok` outcome; a child that could not be created is a
    /// [`RunnerError::Spawn`](crate::runner::RunnerError::Spawn).
    pub started: bool,
    /// The child's exit code. `None` when a signal ended it.
    pub exit_code: Option<i32>,
    /// The timeout fired: the process group was killed, or — where the shell
    /// detaches — the run was moved to the background (`moved_to_background`).
    pub timed_out: bool,
    /// The run's cancellation fired and the process group was killed.
    pub cancelled: bool,
    /// Whether the child ran in the proven sandbox (`true`) or on the host
    /// (`false`) — the path the run actually took, reported so a caller can
    /// say which one it got rather than infer it from the request.
    pub sandboxed: bool,
    /// Either stream hit the output cap; the capture carries what was kept.
    pub truncated: bool,
    /// The redacted, ring-bounded stdout.
    pub stdout: StreamCapture,
    /// The redacted, ring-bounded stderr.
    pub stderr: StreamCapture,
    /// The background id of a run that outlived its timeout and was moved
    /// into the background registry instead of being killed.
    pub moved_to_background: Option<u64>,
    /// The timeout that moved the run, for the notice the model reads.
    pub moved_after: Option<Duration>,
}
