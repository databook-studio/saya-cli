//! The host path's construction: what a `sandbox: false` run may see, and
//! the three ways a shell can come to hold that path. Whether the path
//! exists is the **caller's** decision, taken here and never by the model —
//! [`WorkspaceShell::new`] composes the sandbox alone, [`host_only`] the
//! host alone where no sandbox is proven, [`with_host`] both.
//!
//! The path's confinement is not narrowed, it is *absent*, so the policy is
//! the only thing standing between the child and the caller's own
//! environment: the names in [`HostPolicy::remove_env`] are removed from the
//! inherited environment, and the values in
//! [`HostPolicy::secret_values`] are scrubbed from the capture on top of the
//! `redact()` pass. Both lists are the caller's, built where the caller's
//! secrets are known.
//!
//! [`host_only`]: WorkspaceShell::host_only
//! [`with_host`]: WorkspaceShell::with_host

use std::path::PathBuf;

use crate::runner::RunnerError;

use super::{WorkspaceShell, refused, validate::shell_env};

/// What a host (unsandboxed) run may see. Both lists are the caller's, built
/// where the caller's own secrets are known — the shell never derives them.
#[derive(Debug, Clone, Default)]
pub struct HostPolicy {
    /// Variable names removed from the inherited environment (saya's own
    /// secrets). The host child otherwise inherits this process's whole
    /// environment; these names are the only ones taken away.
    pub remove_env: Vec<String>,
    /// Secret values scrubbed from captured output, in addition to
    /// `redact()`. A bare value carries no marker for the pattern pass to
    /// recognise, so a value the host child can echo is named here.
    pub secret_values: Vec<String>,
}

impl WorkspaceShell {
    /// Builds the shell over the host path alone: this host has no proven
    /// sandbox, so every call is a host call and a call that asks for the
    /// sandbox is a [`RunnerError::ShellSandboxUnavailable`]. The root is
    /// canonicalised once, here, so the workdir containment is decided
    /// against the same root the sandboxed path would have used.
    pub fn host_only(workspace_root: PathBuf, host: HostPolicy) -> Result<Self, RunnerError> {
        let root = std::fs::canonicalize(&workspace_root).map_err(|_| {
            refused(format!(
                "the workspace root `{}` does not resolve",
                workspace_root.display()
            ))
        })?;
        let toolchain = super::toolchain::toolchain_dirs_for_process();
        Ok(Self {
            spawn: None,
            host: Some(host),
            env: shell_env(&root, &toolchain),
            root,
            toolchain,
            cancellation: saya_agent::CancellationToken::new(),
            background: std::sync::Arc::new(super::BackgroundRuns::default()),
            background_cancellation: None,
            detach_on_timeout: false,
            capture_cap: super::OUTPUT_CAP_BYTES,
        })
    }

    /// Opens the host path on a shell that already has the sandbox path:
    /// both become callable, and neither is taken away.
    pub fn with_host(mut self, host: HostPolicy) -> Self {
        self.host = Some(host);
        self
    }

    /// Whether this shell holds the proven-sandbox path.
    pub fn has_sandbox(&self) -> bool {
        self.spawn.is_some()
    }

    /// Whether this shell allows a call to ask for the host path.
    pub fn allows_host(&self) -> bool {
        self.host.is_some()
    }
}
