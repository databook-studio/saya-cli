//! The path decision: which of the composed paths a call takes, resolved once
//! and read by every path that runs or starts a call.

use super::WorkspaceShell;
use super::call::ShellCall;
use super::spawn::ExecTarget;
use crate::runner::RunnerError;

impl WorkspaceShell {
    /// The path this call takes, and only ever a path this shell holds: a
    /// call that names a path the caller did not compose is refused here
    /// rather than served by the other one.
    pub(super) fn resolve_lane(&self, call: &ShellCall) -> Result<bool, RunnerError> {
        match (call.sandbox, self.has_sandbox(), self.allows_host()) {
            (true, true, _) => Ok(true),
            (true, false, _) => Err(RunnerError::ShellSandboxUnavailable),
            (false, _, true) => Ok(false),
            (false, _, false) => Err(RunnerError::ShellHostNotAllowed),
        }
    }

    /// The resolved mode as the spawn path's one target: the proven sandbox or
    /// the host policy, whichever [`resolve_lane`](Self::resolve_lane) named.
    pub(super) fn exec_target(&self, sandboxed: bool) -> ExecTarget<'_> {
        if sandboxed {
            ExecTarget::Sandboxed {
                spawn: self
                    .spawn
                    .as_ref()
                    .expect("the sandbox lane was just checked"),
                env: &self.env,
            }
        } else {
            let host = self.host.as_ref().expect("the host lane was just checked");
            ExecTarget::Host {
                remove_env: &host.remove_env,
                secret_values: &host.secret_values,
            }
        }
    }
}
